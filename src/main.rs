extern crate lazy_static;
extern crate dotenv;
extern crate reqwest;
extern crate serde_json;
extern crate chrono;
extern crate chrono_tz;
extern crate uuid;
extern crate clokwerk;

use std::env;
use std::fs;
use std::io::Write;

use clokwerk::{AsyncScheduler, TimeUnits};

use dotenv::dotenv;

use futures::executor::block_on;

use lazy_static::lazy_static;

use tide::Request;

use chrono::{NaiveDate, DateTime, NaiveDateTime, Utc, TimeZone, Duration};
use crate::chrono_tz::OffsetComponents;

const NOTION_VERSION: &str = "2022-06-28";

lazy_static! {
    static ref NOTION_API_KEY: String = env::var("NOTION_API_KEY")
        .expect("Notion API key is available");

    static ref NOTION_DATABASE_ID: String = env::var("NOTION_DATABASE_ID")
        .expect("Notion Database ID is populated");
}

enum DateObject {
    Date(NaiveDate),
    DateTime(NaiveDateTime)
}

impl DateObject {
    fn now() -> DateObject {
        DateObject::DateTime(Utc::now().naive_utc())
    }
}

#[async_std::main]
async fn main() -> tide::Result<()> {
    println!("Starting app!");
    dotenv().ok();

    let mut app = tide::new();

    if !std::path::Path::new("/tmp/temporary_file.ics").exists() {
        update_calendar().await;
    }

    let mut scheduler = AsyncScheduler::new();

    scheduler.every(30.minutes()).run(|| block_on(update_calendar()));

    tokio::spawn(async move {
        loop {
            scheduler.run_pending().await;
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    });

    let url = env::var("SECRET_URL").unwrap_or("super-secret-url-you-will-never-guess".to_string());
    app.at(&format!("/{}", url)).get(get_ical);

    println!("Started listening on port :8080!");
    app.listen("0.0.0.0:8080").await?;
    Ok(())
}

async fn get_title_and_date(client: &reqwest::Client, entry: &serde_json::Value) -> Option<(String, Vec<serde_json::Value>)> {
    let url = format!(
        "https://api.notion.com/v1/pages/{}/properties/title",
        entry.get("id").unwrap().as_str().unwrap()
    );

    let response: serde_json::Value = setup_request(client.get(url))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let has_date_in_title = &response
        .get("results")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .any(
            |result| {
                let is_mention = result
                    .get("title")
                    .unwrap()
                    .get("type")
                    .unwrap() == "mention";

                is_mention && result
                    .get("title")
                    .unwrap()
                    .get("mention")
                    .unwrap()
                    .get("type")
                    .unwrap() == "date"
            }
        );

    if !has_date_in_title {
        return None
    }

    let title = &response
        .get("results")
        .unwrap()
        .as_array()
        .unwrap()
        [0]
        .get("title")
        .unwrap()
        .get("plain_text")
        .unwrap()
        .as_str()
        .unwrap()
        .trim();

    let dates: Vec<serde_json::Value> = response
        .get("results")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .filter(
            |result| {
                let is_mention = result
                    .get("title")
                    .unwrap()
                    .get("type")
                    .unwrap() == "mention";

                is_mention && result
                    .get("title")
                    .unwrap()
                    .get("mention")
                    .unwrap()
                    .get("type")
                    .unwrap() == "date"
            }
        )
        .map(
            |result| {
                result
                    .get("title")
                    .unwrap()
                    .get("mention")
                    .unwrap()
                    .get("date")
                    .unwrap()
                    .to_owned()
            }
        )
        .collect();

    println!("Got date information for \"{}\"", title.to_string());

    Some((title.to_string(), dates))
}

fn setup_request(request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    request
        .header("Authorization", format!("Bearer {}", NOTION_API_KEY.to_string()))
        .header("Notion-Version", NOTION_VERSION)
        .header("Content-Type", "application/json")
}

async fn get_all_notion_events() -> Result<Vec<(String, Vec<serde_json::Value>)>, reqwest::Error> {
    println!("Getting all Notion events from database");
    let client = reqwest::Client::new();
    let url = format!("https://api.notion.com/v1/databases/{}/query", NOTION_DATABASE_ID.to_string());
    let response: serde_json::Value = setup_request(client.post(url))
        .send()
        .await?
        .json()
        .await?;

    let mut vector: Vec<(String, Vec<serde_json::Value>)> = vec![];
    for entry in response.get("results").unwrap().as_array().unwrap() {
        match get_title_and_date(&client, entry).await {
            Some(value) => vector.push(value),
            None => {}
        };
    }

    Ok(vector)
}

fn parse_date(date_object: &serde_json::Value, is_end: bool) -> Option<DateObject> {
    let string = match date_object.as_str() {
        Some(string) => string,
        None => return None
    };

    let is_date = string.chars().count() <= 10;

    println!("{}", string);

    // ;VALUE=DATE:20220111
    let result = if is_date {
        DateObject::Date(NaiveDate::parse_from_str(string, "%Y-%m-%d")
            .unwrap() + Duration::days(is_end.then_some(1).unwrap_or(0)))
    // :20221211T230000Z
    } else {
        let datetime = DateTime::parse_from_rfc3339(string).unwrap();
        let local_datetime = chrono_tz::Europe::Amsterdam.timestamp(datetime.timestamp(), 0);

        let offset_in_seconds = local_datetime.offset().dst_offset().num_seconds() +
            local_datetime.offset().base_utc_offset().num_seconds();

        DateObject::DateTime(datetime.naive_utc() + Duration::seconds(offset_in_seconds))
    };

    Some(result)
}

async fn get_ical(_request: Request<()>) -> tide::Result {
    println!("Got a request!");
    Ok(fs::read_to_string("/tmp/temporary_file.ics").unwrap().into())
}

fn to_ical_date(date_object: DateObject) -> String {
    match date_object {
        DateObject::Date(date) => date.format(";VALUE=DATE:%Y%m%d").to_string(),
        DateObject::DateTime(datetime) => datetime.format(":%Y%m%dT%H%M%S").to_string()
    }
}

async fn update_calendar() {
    println!("Updating calendar!");
 
    let mut agenda = format!(
        "BEGIN:VCALENDAR
PRODID:-//Bram Dingelstad//Bram's Notion Derived Calendar//EN
CALSCALE:GREGORIAN
VERSION:2.0
METHOD:PUBLISH
X-WR-CALNAME:{calendar_name}
X-WR-TIMEZONE:{timezone}\n",
        calendar_name = "Bram's Notion Derived Calendar",
        timezone = "Europe/Amsterdam"
    );

    for event in get_all_notion_events().await.unwrap() {
        for date in event.1 {
            
            let start = parse_date(date.get("start").unwrap(), false).unwrap();
            let start_as_end = parse_date(date.get("start").unwrap(), true).unwrap();
            let end = match date.get("end") {
                Some(object) => match parse_date(object, true) { Some(date) => date, None => start_as_end },
                None => {
                    start_as_end
                }
            };

            agenda += &format!(
                "BEGIN:VEVENT
DTSTART{start}
DTEND{end}
DTSTAMP{start}
UID:{uuid}@dingelstad.works
CREATED{created}
DESCRIPTION:{description}
LAST-MODIFIED{last_modified}
LOCATION:{location}
SEQUENCE:0
STATUS:CONFIRMED
SUMMARY:{title}
TRANSP:OPAQUE
END:VEVENT\n",
                title = event.0,
                start = to_ical_date(start),
                end = to_ical_date(end),
                uuid = uuid::Uuid::new_v4(),
                created = to_ical_date(DateObject::now()),
                last_modified = to_ical_date(DateObject::now()),
                description = "",
                location = ""
            )

        }
    }

    agenda += "\nEND:VCALENDAR";

    let mut file = std::fs::File::create("/tmp/temporary_file.ics").unwrap();
    write!(file, "{}", agenda).unwrap();
}
