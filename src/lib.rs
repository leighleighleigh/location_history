#![allow(missing_docs)]
//! Library to parse google location history data

use chrono::{DateTime, FixedOffset};
use serde_derive::{Deserialize, Serialize};

extern crate prettytable;
use colored::{Colorize, ColoredString};
use prettytable::row;

extern crate struson;
use std::collections::{HashSet, HashMap};
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use struson::json_path;
use struson::reader::{JsonReader, JsonStreamReader};

use glob_match::glob_match;

#[allow(unused_imports)]
use log::{debug, error, info, log_enabled, Level};

use geo::{Coord, HaversineDistance, Point};

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
    // uses glob_match to compare the activity type - so you can write:
    // {ON_FOOT,STILL} # for either case
    // ON_*            # for whenever we are on something
    // IN_*            # for whenever we are IN something
    fn filter_by_activity(self, activity: String) -> Locations;

    // retrieves the unique set of activity types in the data
    fn list_activities(&self) -> Vec<String>;

    // filters to points within a distance of a point
    fn filter_by_distance(self, point: Point<f64>, distance: f64) -> Locations;
}

impl LocationsExt for Locations {
    fn average_time(&self) -> i64 {
        let mut time = 0;
        for i in 1..self.len() {
            time += self[i].timestamp.timestamp() - self[i - 1].timestamp.timestamp()
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
            if let Some(speed) = location.speed_kmh(&tmp[tmp.len() - 1]) {
                if speed < 300.0 {
                    tmp.push(location);
                }
            } else {
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
                    if let Some(activity) = activity.activities.iter().max_by_key(|x| x.confidence)
                    {
                        if glob_match(&activity_type, &activity.activity_type) {
                            tmp.push(location.clone());
                            break;
                        }
                    }
                }
            }
        }
        tmp.sort_chronological();
        tmp
    }

    fn list_activities(&self) -> Vec<String> {
        // make hashmap for efficiency
        let mut activities_set: HashSet<String> = HashSet::new();

        for location in self.into_iter() {
            // iterate through all activities recorded at this location
            if let Some(activities) = &location.activities {
                for activity in activities.into_iter() {
                    for act in activity.activities.iter() {
                        activities_set.insert(act.activity_type.clone());
                    }
                }
            }
        }

        activities_set.into_iter().collect::<Vec<String>>()
    }

    fn filter_by_distance(self, point: Point<f64>, distance: f64) -> Locations {
        let mut tmp: Vec<Location> = Vec::new();

        for location in self.iter() {
            let lp: Point<f64> = (&location.clone()).into();
            if lp.haversine_distance(&point) < distance {
                tmp.push(location.clone());
            }
        }
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
        match tx.send(location) {
            Ok(_) => {}
            Err(_) => break,
        }
    }
}

// make an activity type Enum, which will be useful for color-coding and filtering things by activity
#[derive(Debug,Clone,Copy,PartialEq,Eq,Hash)]
#[allow(non_camel_case_types)]
pub enum ActivityType {
    IN_VEHICLE,
    EXITING_VEHICLE,
    ON_BICYCLE,
    ON_FOOT,
    RUNNING,
    STILL,
    TILTING,
    UNKNOWN,
    WALKING,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Activity {
    #[serde(rename = "type")]
    pub activity_type: String,
    pub confidence: i32,
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
    pub latitude: f64,
    #[serde(rename = "longitudeE7", deserialize_with = "parse_location")]
    /// longitude, converted from long E7
    pub longitude: f64,
    /// accuracy of location sample in meters
    pub accuracy: Option<i32>,
    /// altitude in meters, if available
    pub altitude: Option<i32>,

    #[serde(rename = "activity")]
    pub activities: Option<Vec<Activities>>,
}

impl Location {
    /// calculate the haversine distance between this and another location.
    /// now uses the geo crate!
    pub fn haversine_distance(&self, other: &Location) -> f64 {
        let p1: Point<f64> = self.into();
        let p2: Point<f64> = other.into();
        p1.haversine_distance(&p2)
    }

    /// calculate the speed in km/h from this location to another location
    /// if a maximum time delta is exceeded, None is returned
    pub fn speed_kmh(&self, other: &Location) -> Option<f64> {
        let dist = self.haversine_distance(other);
        let time = self.timestamp.timestamp() - other.timestamp.timestamp();

        // 10 minute gap
        if time > 0 && time < 600 {
            let meter_second = dist / time as f64;
            Some(meter_second * 3.6)
        } else {
            None
        }
    }

    pub fn top_activities(&self) -> Vec<Activity> {
        // makes a flat list of activities, in descending order.
        // we cant simply store the top activity - as quite frequently, there will be multiple
        // items of equal value. 
        let mut result : Vec<Activity> = Vec::new();

        if let Some(acts) = &self.activities {
            for activity_vec in acts {
                result.extend(activity_vec.top_activities());
            }
        }

        // sort the super list
        result.sort_by(|a, b| b.confidence.cmp(&a.confidence));
        result
    }

    pub fn merged_activities(&self) -> Activities {
        // merge all activities into a single list
        let mut result = Activities {
            timestamp: self.timestamp,
            activities: Vec::new(),
        };

        let mut all_activities : HashMap<ActivityType, i32> = HashMap::new();

        if let Some(acts) = &self.activities {
            for activity_vec in acts {
                // make a hashmap
                let activities : HashMap<ActivityType, i32> = activity_vec.into();
                
                // add to all_activities, summing the confidence
                // add the confidences to the hashmap
                for (k, v) in activities.iter() {
                    *all_activities.entry(*k).or_insert(0) += v;
                }

            }
        }

        // convert to a vector, using the hashmap as a guide
        let mut activities : Vec<Activity> = Vec::new();
        for (act_type, confidence) in all_activities.iter() {
            activities.push(Activity {
                activity_type: act_type.into(),
                confidence: confidence.clone(),
            });
        }

        // sort the list by confidence
        activities.sort_by(|a, b| b.confidence.cmp(&a.confidence));
        result.activities = activities;

        result
    }

}

impl Activities {
    pub fn top_activity(&self) -> Activity {
        let act = self.top_activities();

        if act.len() > 0 {
            act[0].clone()
        } else {
            Activity {
                activity_type: "UNKNOWN".to_string(),
                confidence: 0,
            }
        }
    }

    pub fn top_activities(&self) -> Vec<Activity> {
        // makes a flat list of activities, in descending order.
        // we cant simply store the top activity - as quite frequently, there will be multiple
        // items of equal value. 
        let mut result : Vec<Activity> = Vec::new();

        // make a hashmap
        let activities : HashMap<ActivityType, i32> = self.into();
        
        // convert to a vector, using the hashmap as a guide
        for (act_type, confidence) in activities.iter() {
            result.push(Activity {
                activity_type: act_type.into(),
                confidence: confidence.clone(),
            });
        }

        // sort the list by confidence
        result.sort_by(|a, b| b.confidence.cmp(&a.confidence));
        result
    }

    pub fn top_activity_type(&self) -> ActivityType {
        let act = self.top_activity();
        act.into()
    }

    pub fn seconds_delta(&self, other: &Activities) -> i64 {
        self.timestamp.timestamp() - other.timestamp.timestamp()
    }

    pub fn is_similar_type(&self, other: &Activities) -> bool {
        // if our top activity is within the top 3 of the other top activities, ignoring time delta
        let top_act = self.top_activity();
        let other_top_act : Vec<Activity> = other.top_activities().into_iter().take(3).collect();

        for act in other_top_act {
            if act.activity_type == top_act.activity_type {
                return true;
            }
        }
        false
    }

}

// impliment to HashMap conversion of Activities- this is useful for grouping activities by type, and summing them.
impl Into<HashMap<ActivityType, i32>> for &Activities {
    fn into(self) -> HashMap<ActivityType, i32> {
        let mut result: HashMap<ActivityType, i32> = HashMap::new();

        for act in self.activities.iter() {
            let act_type: ActivityType = act.clone().into();
            let act_confidence: i32 = act.confidence.clone();

            // if we already have this activity type, add the confidence to it
            if result.contains_key(&act_type) {
                let current_confidence = result.get(&act_type).unwrap();
                result.insert(act_type, current_confidence + act_confidence);
            } else {
                // otherwise, add it to the hashmap
                result.insert(act_type, act_confidence);
            }
        }

        result
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
        serde_json::Value::String(ref s) => Ok(DateTime::parse_from_rfc3339(s).unwrap()),
        _ => Err(serde::de::Error::custom("Unexpected value")),
    }
}

fn parse_location<'de, D>(de: D) -> Result<f64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let deser_result: serde_json::Value = serde::Deserialize::deserialize(de)?;
    match deser_result {
        serde_json::Value::Number(ref i) => Ok((i.as_f64().unwrap() / 10_000_000.0) as f64),
        _ => Err(serde::de::Error::custom("Unexpected value")),
    }
}

// convert location into a Point
impl Into<Point<f64>> for &Location {
    fn into(self) -> Point<f64> {
        let c: Coord<f64> = self.into();
        Point::from(c)
    }
}

impl Into<Coord<f64>> for &Location {
    fn into(self) -> Coord<f64> {
        Coord {
            x: self.latitude as f64,
            y: self.longitude as f64,
        }
    }
}

// impliment nicer-looking Display traits for Location, Locations, Activity, and Activities
impl std::fmt::Display for Location {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        // show all fields of Location in a condensed manner, using prettytable
        let mut table = prettytable::Table::new();

        // show the data in a key : value format, with the key in bold
        table.add_row(row!["timestamp".bold(), self.timestamp]);

        // old - show lat/long separately
        // table.add_row(row!["latitude".bold(), self.latitude]);
        // table.add_row(row!["longitude".bold(), self.longitude]);
        // new - show as Point
        let pt: Point<f64> = self.into();
        table.add_row(row!["location".bold(), format!("{:#?}", pt.0)]);

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
                activities
                    .iter()
                    .map(|x| x.to_string())
                    .collect::<Vec<String>>()
                    .join("\n")
            }
            None => "None".to_string(),
        };
        table.add_row(row!["activities".bold(), activity_str]);

        // write the table to the formatter
        table.fmt(f)
    }
}

impl From<String> for ActivityType {
    fn from(value : String) -> ActivityType {
        let a : Activity = Activity { activity_type : value, confidence : 0 };
        a.into()
    }
}

impl Into<ActivityType> for Activity {
    fn into(self) -> ActivityType {
        match self.activity_type.as_str() {
            "IN_VEHICLE" => ActivityType::IN_VEHICLE,
            "EXITING_VEHICLE" => ActivityType::EXITING_VEHICLE,
            "ON_BICYCLE" => ActivityType::ON_BICYCLE,
            "ON_FOOT" => ActivityType::ON_FOOT,
            "RUNNING" => ActivityType::RUNNING,
            "STILL" => ActivityType::STILL,
            "TILTING" => ActivityType::TILTING,
            "UNKNOWN" => ActivityType::UNKNOWN,
            "WALKING" => ActivityType::WALKING,
            _ => ActivityType::UNKNOWN,
        }
    }
}

impl Into<String> for &ActivityType {
    fn into(self) -> String {
        match self {
            ActivityType::IN_VEHICLE => "IN_VEHICLE".to_string(),
            ActivityType::EXITING_VEHICLE => "EXITING_VEHICLE".to_string(),
            ActivityType::ON_BICYCLE => "ON_BICYCLE".to_string(),
            ActivityType::ON_FOOT => "ON_FOOT".to_string(),
            ActivityType::RUNNING => "RUNNING".to_string(),
            ActivityType::STILL => "STILL".to_string(),
            ActivityType::TILTING => "TILTING".to_string(),
            ActivityType::UNKNOWN => "UNKNOWN".to_string(),
            ActivityType::WALKING => "WALKING".to_string(),
        }
    }
}

// convert activity types into single characters! :)
// this makes it easy to see sequences of related activities
impl Into<ColoredString> for ActivityType {
    fn into(self) -> ColoredString {
        match self {
            ActivityType::IN_VEHICLE => "$".to_string().bright_blue().on_blue(),
            ActivityType::EXITING_VEHICLE => "^".to_string().bright_blue().on_blue(),
            ActivityType::ON_FOOT => "#".to_string().bright_green().on_green(),
            ActivityType::WALKING => "#".to_string().bright_green().on_green(),
            ActivityType::RUNNING => "#".to_string().green().on_bright_green(),
            ActivityType::ON_BICYCLE => "%".to_string().bright_yellow().on_yellow(),
            ActivityType::STILL => ".".to_string().white(),
            ActivityType::TILTING => "/".to_string().dimmed(),
            ActivityType::UNKNOWN => "?".to_string().dimmed(),
        }
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
