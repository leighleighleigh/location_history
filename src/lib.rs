#![allow(missing_docs)]
//! Library to parse google location history data

use chrono::{DateTime, FixedOffset};
use serde_derive::{Deserialize, Serialize};

extern crate prettytable;
use prettytable::row;
use colored::Colorize;

extern crate struson;
use struson::reader::{JsonStreamReader,JsonReader};
use struson::json_path;
use std::io::BufReader;
use std::sync::mpsc::Sender;
use std::fs::File;
use std::path::PathBuf;

/// group of locations
pub type Locations = Vec<Location>;

/// methods used for locations
pub trait LocationsExt {
    /// calculate average time between locations
    fn average_time(&self) -> i64;

    /// find the closest Location to a datetime
    fn find_closest(&self, time: DateTime<FixedOffset>) -> Option<Location>;

    /// sort locations by timestamp
    fn sort_chronological(&mut self);

    /// remove locations that are offset more than 300km/h from last location
    fn filter_outliers(self) -> Locations;

    // filter by activity, where the highest-confidence activity is the one that is passed as argument
    fn filter_by_activity(self, activity: String) -> Locations;
}

impl LocationsExt for Locations {
    fn average_time(&self) -> i64 {
        let mut time = 0;
        for i in 1..self.len() {
            time += self[i - 1].timestamp.timestamp() - self[i].timestamp.timestamp()
        }
        time / (self.len() as i64)
    }

    fn find_closest(&self, time: DateTime<FixedOffset>) -> Option<Location> {
        let result = self.binary_search_by(|x| x.timestamp.cmp(&time));
        let index = match result {
            Ok(x) => Some(x),
            // if this is 0 or the len of locations return None
            Err(x) => {
                if x > 0 && x < self.len() {
                    Some(x)
                } else {
                    None
                }
            }
        };

        if let Some(x) = index {
            if x < self.len() {
                return Some(self[x].clone());
            }
        }
        None
    }

    fn sort_chronological(&mut self) {
        self.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
    }

    fn filter_outliers(self) -> Locations {
        let mut tmp: Vec<Location> = vec![self[0].clone()];
        for location in self.into_iter() {
            if location.speed_kmh(&tmp[tmp.len() - 1]) < 300.0 {
                tmp.push(location);
            }
        }
        tmp.sort_chronological();
        tmp
    }

    fn filter_by_activity(self, activity_type: String) -> Locations {
        let mut tmp: Vec<Location> = Vec::new();

        for location in self.into_iter() {
            // iterate through all activities recorded at this location
            if let Some(activities) = &location.activities {
                for activity in activities.into_iter() {
                    // check if the highest-confidence activity is the one we want
                    if let Some(activity) = activity.activities.iter().max_by_key(|x| x.confidence) {
                        if activity.activity_type == activity_type {
                            tmp.push(location.clone());
                            // we only want to add each location once
                            break;
                        }
                    }
                }
            }
        }
        tmp.sort_chronological();
        tmp
    }
}

/// deserialize location history
pub fn deserialize(from: &str) -> Locations {
    #[derive(Deserialize)]
    struct LocationList {
        locations: Vec<Location>,
    }

    let mut deserialized: LocationList = serde_json::from_str(from).expect("Failed to deserialize");

    deserialized
        .locations
        .sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
    deserialized.locations
}


/// Reads a `Records.json` file and decodes the data on-the-fly.
/// The file is expected to contain a single large array of `Location` objects
/// under a 'locations' key.
///
/// This function sends each decoded `Location` object to the provided
/// MPSC channel as soon as it is decoded.
///
/// It is recommended to call this function from a separate thread, as it will
/// block until the entire file has been read.
///
/// # Arguments
///
/// * `from` - The path to the `Records.json` file.
/// * `tx` - The `Sender` channel to send the decoded `Location` objects.
pub fn deserialize_streaming(from: PathBuf, tx: Sender<Location>) {
    let file = File::open::<PathBuf>(from).unwrap();
    let reader = BufReader::new(file);

    let mut json_reader = JsonStreamReader::new(reader);

    json_reader.seek_to(&json_path!["locations"]).unwrap();

    json_reader.begin_array().unwrap();

    while json_reader.has_next().unwrap() {
        let location: Location = json_reader.deserialize_next().unwrap();
        tx.send(location).unwrap();
    }

    // Optionally consume the remainder of the JSON document
    json_reader.end_array().unwrap();
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Activity {
    #[serde(rename = "type")]
    pub activity_type : String,
    pub confidence : i32,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Activities {
    #[serde(deserialize_with = "parse_timestamp")]
    /// timestamp this location was sampled at
    pub timestamp: DateTime<FixedOffset>,
    
    /// activities list
    #[serde(rename = "activity")]
    pub activities: Vec<Activity>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
/// Location sample parsed from LocationHistory.json
pub struct Location {
    #[serde(deserialize_with = "parse_timestamp")]
    /// timestamp this location was sampled at
    pub timestamp: DateTime<FixedOffset>,
    #[serde(rename = "latitudeE7", deserialize_with = "parse_location")]
    /// latitude, converted from lat E7
    pub latitude: f32,
    #[serde(rename = "longitudeE7", deserialize_with = "parse_location")]
    /// longitude, converted from long E7
    pub longitude: f32,
    /// accuracy of location sample in meters
    pub accuracy: Option<i32>,
    /// altitude in meters, if available
    pub altitude: Option<i32>,
    
    #[serde(rename = "activity")]
    pub activities: Option<Vec<Activities>>,
}

impl Location {
    /// calculate the haversine distance between this and another location
    pub fn haversine_distance(&self, other: &Location) -> f32 {
        let long1 = self.longitude.to_radians();
        let long2 = other.longitude.to_radians();
        let lat1 = self.latitude.to_radians();
        let lat2 = other.latitude.to_radians();
        let dlon = long2 - long1;
        let dlat = lat2 - lat1;
        let a = (dlat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (dlon / 2.0).sin().powi(2);
        let c = 2.0 * a.sqrt().asin();
        c * 6_371_000.0
    }

    /// calculate the speed in km/h from this location to another location
    pub fn speed_kmh(&self, other: &Location) -> f32 {
        let dist = self.haversine_distance(other);
        let time = self.timestamp.timestamp() - other.timestamp.timestamp();
        if time > 0 {
            let meter_second = dist / time as f32;
            meter_second * 3.6
        } else {
            dist / 1000.0
        }
    }
}

fn parse_timestamp<'de, D>(de: D) -> Result<DateTime<FixedOffset>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    // Expects the new Records.json timestamp format, e.g:
    //     "timestamp": "2016-08-07T04:54:00.678Z"
    let deser_result: serde_json::Value = serde::Deserialize::deserialize(de)?;

    match deser_result {
        serde_json::Value::String(ref s) => Ok(DateTime::parse_from_rfc3339(
            s
        ).unwrap()),
        _ => Err(serde::de::Error::custom("Unexpected value")),
    }
}

fn parse_location<'de, D>(de: D) -> Result<f32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let deser_result: serde_json::Value = serde::Deserialize::deserialize(de)?;
    match deser_result {
        serde_json::Value::Number(ref i) => Ok((i.as_f64().unwrap() / 10_000_000.0) as f32),
        _ => Err(serde::de::Error::custom("Unexpected value")),
    }
}


// impliment nicer-looking Display traits for Location, Locations, Activity, and Activities
impl std::fmt::Display for Location {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        // show all fields of Location in a condensed manner, using prettytable
        let mut table = prettytable::Table::new();

        // show the data in a key : value format, with the key in bold
        table.add_row(row!["timestamp".bold(), self.timestamp]);
        table.add_row(row!["latitude".bold(), self.latitude]);
        table.add_row(row!["longitude".bold(), self.longitude]);

        // map accuracy to ??? if unknown
        let accuracy_str = match self.accuracy {
            Some(accuracy) => accuracy.to_string(),
            None => "???".to_string(),
        };
        table.add_row(row!["accuracy".bold(), accuracy_str]);

        // map altitude to ??? if unknown
        let altitude_str = match self.altitude {
            Some(altitude) => altitude.to_string(),
            None => "???".to_string(),
        };
        table.add_row(row!["altitude".bold(), altitude_str]);

        // map the activities to a string
        let activity_str = match &self.activities {
            Some(activities) => {
                // sort by timestamp
                let mut activities = activities.clone();
                activities.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
                // join each activity vec by newline, and each activity by a comma
                activities.iter().map(|x| x.to_string()).collect::<Vec<String>>().join("\n")
            }
            None => "None".to_string(),
        };
        table.add_row(row!["activities".bold(), activity_str]);

        // write the table to the formatter
        table.fmt(f)
    }
}

impl std::fmt::Display for Activity {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{:<16}({:>3}%)", self.activity_type, self.confidence)
    }
}

impl std::fmt::Display for Activities {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}\n", self.timestamp)?;

        for act in self.activities.iter() {
            write!(f, "{}\n", act)?;
        }
        Ok(())
    }
}


#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        use crate::LocationsExt;

        let test_data = r#"{"locations" : [ {
                            "timestamp" : "2016-08-07T04:54:00.678Z",
                            "latitudeE7" : 500373489,
                            "longitudeE7" : 83320934,
                            "accuracy" : 19,
                            "activitys" : [ {
                                "timestamp" : "2016-08-07T04:54:00.678Z",
                                "activities" : [ {
                                    "type" : "still",
                                    "confidence" : 100
                                } ]
                                }, {
                                "timestamp" : "2016-08-07T04:54:00.678Z",
                                "activities" : [ {
                                "type" : "still",
                                "confidence" : 100
                                } ]
                            } ]
                            }]}"#;
        let _locations = crate::deserialize(&test_data).filter_outliers();
    }
}
