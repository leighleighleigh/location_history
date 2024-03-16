#![feature(linked_list_cursors)]

use anyhow::Result;
use chrono::{DateTime, Local, NaiveDate, TimeZone, Datelike};
use itertools::{Itertools,max};

use geo::{Coord, Point};
const MEAN_EARTH_RADIUS: f64 = 6371008.8;

use colored::{ColoredString, Colorize};
use spinner::SpinnerBuilder;

use std::collections::{LinkedList, HashMap};
use std::path::PathBuf;
use std::sync::mpsc::channel;
use std::thread;

extern crate rerun;

#[allow(unused_imports)]
use log::{debug, error, info, log_enabled, Level};

#[allow(unused_imports)]
use textplots::{AxisBuilder, Chart, Plot, Shape};

extern crate location_history;
use location_history::{ActivityType, Location, LocationsExt, Activities, Activity};

use clap::Parser;

#[derive(Parser)]
#[command(name = "location-history")]
#[command(bin_name = "location-history")]
enum LocationHistoryCLI {
    Load(LoadArgs),
}

#[derive(clap::Args)]
#[command(author, version, about, long_about = None)]
struct LoadArgs {
    #[arg(short = 's')]
    start_date: Option<String>,
    #[arg(short = 'e')]
    end_date: Option<String>,
    #[arg(short = 'a')]
    activity_type: Option<String>,

    #[clap(short = 'c', number_of_values = 3, allow_hyphen_values = true)]
    center_point_radius: Option<Vec<f64>>,

    #[arg(short = 'n')]
    record_limit: Option<usize>,

    #[arg(short = 'w', default_value = "30", help = "activity window in minutes")]
    activity_window : Option<i64>,

    // flag for rerun logging
    #[arg(short = 'r', default_value = "false")]
    rerun: bool,

    records_json_path: PathBuf,
}

// Function to convert geographic coordinates to local east-north-up coordinates
fn convert_to_enu(coord: Coord<f64>, center_coord: Coord<f64>) -> Coord<f64> {
    // Convert latitude and longitude to radians
    let lat_rad = coord.x.to_radians();
    let lon_rad = coord.y.to_radians();

    // Center coordinates in radians
    let center_lat_rad = center_coord.x.to_radians();
    let center_lon_rad = center_coord.y.to_radians();

    // Calculate true northward and eastward distances
    let northward = (lat_rad - center_lat_rad) * MEAN_EARTH_RADIUS;
    let eastward = (lon_rad - center_lon_rad) * MEAN_EARTH_RADIUS * lat_rad.cos();

    // Upward is assumed to be zero for a flat-plane approximation

    Coord::from((eastward, northward))
}

// Function to convert latitude and longitude to X/Y/Z on the globe surface
fn convert_to_xyz(loc: &Location, center_point_radius: &Option<Vec<f64>>) -> (f64, f64, f64) {
    let coord: Coord<f64> = loc.into();
    let altitude = loc.altitude.unwrap_or(0) as f64;

    // if a centerpoint and radius is provided, we calculate the cartesian distance in meters (north / east) from the centerpoint
    if let Some(ref center_point_radius) = center_point_radius {
        let center_lat = center_point_radius[0];
        let center_long = center_point_radius[1];

        let center_coord: Coord<f64> = (center_lat, center_long).into();

        // convert the coord into east-north-up coordinates, assuming a flat plane, relative to the center point.
        // this first requires a projection from lat/long to a flat plane, then a conversion to east-north-up

        let enu_dist = convert_to_enu(coord, center_coord);

        return (enu_dist.x, -enu_dist.y, altitude);
    } else {
        // convert into cartesian coordinates, using geo
        let (x, y) = coord.x_y();
        // flip the latitude/longitude so that they look correct for south hemisphere, with north-up
        (y, -x, altitude)
    }
}

fn main() -> Result<()> {
    // env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    env_logger::init();

    let LocationHistoryCLI::Load(args) = LocationHistoryCLI::parse();

    // parse start_date and end_date, if provided. assume the format is yy_mm_dd, and is provided in our local timezone
    let start_date: Option<DateTime<Local>> = args.start_date.map(|s| {
        let dt = NaiveDate::parse_from_str(&s, "%y_%m_%d").unwrap();
        Local
            .from_local_datetime(&dt.and_hms_opt(0, 0, 0).unwrap())
            .unwrap()
    });

    let end_date: Option<DateTime<Local>> = args.end_date.map(|s| {
        let dt = NaiveDate::parse_from_str(&s, "%y_%m_%d").unwrap();
        Local
            .from_local_datetime(&dt.and_hms_opt(0, 0, 0).unwrap())
            .unwrap()
    });

    // a background thread performs streaming deserialization, while the main thread
    // handles the Locations as they are deserialized. filtering is performed in the main thread.
    let (tx, rx) = channel();
    let mut locations: Vec<Location> = Vec::new();
    let mut locations_count: u64 = 0;

    // spawn a thread to read the json file 'in the background'
    let reader_jh = thread::spawn(move || {
        location_history::deserialize_streaming(args.records_json_path, tx);
    });

    // main thread handles the Locations as they are deserialized
    let sp = SpinnerBuilder::new("Loading data...".into()).start();

    for loc in rx {
        locations_count += 1;

        sp.update(format!(
            "{} loaded, {} parsed",
            locations.len(),
            locations_count
        ));

        // check if the location is within the date range
        if let Some(start_date) = start_date {
            if loc.timestamp.date_naive() < start_date.date_naive() {
                continue;
            }
        }
        if let Some(end_date) = end_date {
            if loc.timestamp.date_naive() >= end_date.date_naive() {
                continue;
            }
        }

        // going to store this location.
        locations.push(loc);

        // if the record limit is reached, stop
        if let Some(record_limit) = args.record_limit {
            if locations.len() >= record_limit {
                break;
            }
        }
    }

    sp.message(format!(
        "{} returned, {} parsed",
        locations.len(),
        locations_count
    ));
    sp.close();

    println!();

    // iterate over locations, and print a letter for each one,
    // representing the highest-confidence activity type
    let bubble_window : i64 = 60 * args.activity_window.unwrap_or(30);

    // group the entries by month 
    let grouped: Vec<Vec<Location>> = locations
        .iter()
        .group_by(|loc| loc.timestamp.date_naive().month())
        .into_iter()
        .map(|(_, g)| g.cloned().collect())
        .collect();
    

    // GROUP BY MONTH, THEN WEEK, THEN DAY!
    for g in grouped.iter() {
        // print the month
        println!("\n{}",format!("{:<4} {}", g[0].timestamp.format("%Y").to_string(), g[0].timestamp.format("%B").to_string().bold()).black().on_white());

        let by_week : Vec<Vec<Location>> = g.iter()
                                            .group_by(|loc| loc.timestamp.date_naive().iso_week())
                                            .into_iter()
                                            .map(|(_, g)| g.cloned().collect())
                                            .collect();

        
        for week in by_week.iter() {
            // For each week in this month, print the week number
            println!("{:>6} {}","W",format!("{:<02}",week[0].timestamp.iso_week().week()).bold());

            // Then group by day
            let by_day: Vec<Vec<Location>> = week.iter()
                                            .group_by(|loc| loc.timestamp.num_days_from_ce())
                                            .into_iter()
                                            .map(|(_, d)| d.cloned().collect())
                                            .collect();
            
            // then group by day in the week, showing activities
            for day in by_day.iter() {
                print!("{:>10} ", day[0].timestamp.weekday().to_string());

                let mut act_ll : Vec<Activities> = Vec::new();

                for loc in day.iter() {
                    act_ll.push(loc.merged_activities());
                }

                let mut last_act : Option<Activity> = None;
                let mut last_act_type : ActivityType = ActivityType::UNKNOWN;

                for acts in act_ll {
                    let top_act_type = acts.top_activity_type();

                    // if the activity type has not changed, ignore
                    if top_act_type == last_act_type {
                        continue;
                    } else {
                        last_act_type = top_act_type;
                    }
                    
                    match top_act_type {
                        ActivityType::UNKNOWN | ActivityType::STILL | ActivityType::TILTING => {continue}
                        _ => {
                            // print the activity type, after casting to colored string
                            let act_c : ColoredString = top_act_type.into();
                            print!("{}", act_c);
                        }
                    }
                }

                println!();
                // END DAY LOOP
            }

            // END WEEK LOOP
        }

        println!();
        // END MONTH LOOP
    }

    println!();

    // remove high-velocity outliers
    let mut filtered_locations = locations.clone();

    // store length before filtering
    let mut len_before = filtered_locations.len();
    filtered_locations = filtered_locations.filter_outliers();
    let delta: i64 = (len_before - filtered_locations.len()) as i64;
    debug!("Removed {} outliers by velocity", delta);

    if let Some(ref center_point_radius) = args.center_point_radius {
        let lat = center_point_radius[0];
        let long = center_point_radius[1];
        let radius = center_point_radius[2];

        let origin: Point<f64> = Point::new(lat, long);
        filtered_locations = filtered_locations.filter_by_distance(origin, radius);
    }

    // print the list of activities
    println!("{:^32}","LEGEND".black().bold().on_white());
    let activity_list = filtered_locations.list_activities();

    // find the longest activity name
    let name_pad = max(activity_list.clone().into_iter().map(|a| a.len()).collect::<Vec<_>>()).unwrap_or(16);
    
    // two columns
    for (row,chunk) in activity_list.chunks(2).enumerate() {
        for (col, activity) in chunk.iter().enumerate() {
            let act : ActivityType = activity.clone().into();
            let act_c : ColoredString = act.clone().into();
            let pad = col*2;
            print!("{:>pad$} {} {:<name_pad$} ","", act_c, activity);
        }
        println!();
    }

    println!();

    len_before = filtered_locations.len();

    // filter by activity, start and end date
    if let Some(activity_type) = args.activity_type {
        filtered_locations = filtered_locations.filter_by_activity(activity_type.into());
        // store length after filtering
        info!(
            "Removed {} locations by activity type",
            len_before - filtered_locations.len()
        );
    }

    // // print some locations
    // for loc in filtered_locations.iter().take(1) {
    //     info!("\n{}", loc);
    // }

    //if args.rerun {
    //    // group lines by day
    //    let line_groups: Vec<Vec<Location>> = filtered_locations
    //        .iter()
    //        .group_by(|loc| loc.timestamp.timestamp() / 3600)
    //        .into_iter()
    //        .map(|(_, g)| g.cloned().collect())
    //        .collect();

    //    // plot with rerun!
    //    let rec = rerun::RecordingStreamBuilder::new("rerun_example_app").spawn()?;

    //    for group in line_groups {
    //        let time = group[0].timestamp.timestamp() as f64;

    //        // vec of x,y,z (lat long  altitude)
    //        let coords: Vec<(f64, f64, f64)> = group
    //            .iter()
    //            .map(|loc| convert_to_xyz(loc, &args.center_point_radius))
    //            .collect();
    //        let coords_pts: Vec<(f32, f32)> = coords
    //            .iter()
    //            .map(|(x, y, _)| (*x as f32, *y as f32))
    //            .collect();

    //        // make a rerun LineStrips3D object
    //        let linestrip = rerun::LineStrips2D::new(vec![coords_pts]);

    //        rec.set_time_seconds("time", time);
    //        rec.log("position", &linestrip).unwrap();
    //    }
    //}

    // wait for the reader to finish
    reader_jh.join().unwrap();

    Ok(())
}
