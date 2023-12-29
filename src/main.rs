use itertools::Itertools;
use anyhow::Result;
use chrono::{DateTime, Local, NaiveDate, TimeZone, Datelike};

use geo::{Coord,Point};

use spinner::SpinnerBuilder;
use std::path::PathBuf;
use std::sync::mpsc::channel;
use std::thread;

extern crate rerun;

#[allow(unused_imports)]
use log::{debug, error, log_enabled, info, Level};

#[allow(unused_imports)]
use textplots::{AxisBuilder, Chart, Plot, Shape};

extern crate location_history;
use location_history::{Location, LocationsExt};

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
    center_point_radius : Option<Vec<f64>>,

    #[arg(short = 'n')]
    record_limit: Option<usize>,
    records_json_path: PathBuf,
}

// Function to convert latitude and longitude to X/Y/Z on the globe surface
fn convert_to_xyz(loc : &Location) -> (f64, f64, f64) {
    let coord : Coord<f64> = loc.into();
    let altitude = loc.altitude.unwrap_or(0) as f64;

    // convert into cartesian coordinates, using geo
    let (x,y) = coord.x_y();
    // flip the latitude/longitude so that they look correct for south hemisphere, with north-up
    (y,-x,altitude)


    // // Convert latitude and longitude to radians
    // let lat_rad = latitude.to_radians();
    // let lon_rad = longitude.to_radians();
    // // Radius of the Earth in meters
    // let radius = 6371000.0;
    // // Calculate Cartesian coordinates on the globe surface
    // let x = radius * lat_rad.cos() * lon_rad.sin();
    // let y = radius * lat_rad.cos() * lon_rad.cos();
    // let z = radius * lat_rad.sin();
    // (x, y, z)
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

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
            if loc.timestamp <= start_date {
                continue;
            }
        }
        if let Some(end_date) = end_date {
            if loc.timestamp >= end_date {
                continue;
            }
        }

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

    // remove high-velocity outliers
    let mut filtered_locations = locations.clone();

    // store length before filtering
    let mut len_before = filtered_locations.len();
    filtered_locations = filtered_locations.filter_outliers();
    let delta: i64 = (len_before - filtered_locations.len()) as i64;
    info!("Removed {} outliers by velocity", delta);

    if let Some(center_point_radius) = args.center_point_radius {
        let lat = center_point_radius[0];
        let long = center_point_radius[1];
        let radius = center_point_radius[2];

        let origin : Point<f64> = Point::new(lat, long);
        filtered_locations = filtered_locations.filter_by_distance(origin, radius);
    }

    // print the list of activities
    info!("Activities:");
    let activity_list = filtered_locations.list_activities();

    for activity in activity_list {
        info!(" - {}", activity);
    }


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

    // print some locations
    for loc in filtered_locations.iter().take(1) {
        info!("\n{}", loc);
    }

    // group lines by day
    let line_groups : Vec<Vec<Location>> = filtered_locations
        .iter()
        .group_by(|loc| loc.timestamp.timestamp() / 3600)
        .into_iter()
        .map(|(_,g)| g.cloned().collect())
        .collect();

    // plot with rerun!
    let rec = rerun::RecordingStreamBuilder::new("rerun_example_app").spawn()?;

    for group in line_groups {
        let time = group[0].timestamp.timestamp() as f64;

        // vec of x,y,z (lat long  altitude)
        let coords : Vec<(f64,f64,f64)> = group.iter().map(|loc| convert_to_xyz(loc)).collect();
        let mut coords_pts : Vec<(f32,f32)> = coords.iter().map(|(x,y,_)| (*x as f32,*y as f32)).collect();
        // multiply the points by 10 to make them visible
        coords_pts = coords_pts.iter().map(|(x,y)| (*x * 10.0,*y * 10.0)).collect();

        // make a rerun LineStrips3D object
        let linestrip = rerun::LineStrips2D::new(vec![coords_pts]);

        rec.set_time_seconds("time", time);
        rec.log("position", &linestrip).unwrap();
    }

    // for (time,coord) in txy {
    //     rec.set_time_seconds("time", time);
    //     rec.log("position", &rerun::Points3D::new(vec![coord])).unwrap();
    // }


    // let positions: Vec<Coord<f64>> = filtered_locations
    //     .iter()
    //     .map(|loc| loc.into())
    //     .collect();

    // // convert the times to unix epoch seconds/milliseconds
    // let times: Vec<i64> = times.iter().map(|dt| dt.timestamp_millis()).collect();
    // // convert the positions to tuples
    // let xys: Vec<(f64, f64)> = positions.iter().map(|p| (p.x, p.y)).collect();
    // let xs: Vec<f64> = positions.iter().map(|p| (p.x)).collect();
    // let ys: Vec<f64> = positions.iter().map(|p| (p.y)).collect();

    // // get min/max of x and y
    // let min_x = xs.clone().into_iter().reduce(f64::min).unwrap_or(0.0);
    // let max_x = xs.clone().into_iter().reduce(f64::max).unwrap_or(0.0);
    // let min_y = ys.clone().into_iter().reduce(f64::min).unwrap_or(0.0);
    // let max_y = ys.clone().into_iter().reduce(f64::max).unwrap_or(0.0);
    // // use term_size to get the terminal size
    // let (w, h) = term_size::dimensions().unwrap();
    // Chart::new_with_y_range((w*1) as u32, (h*3) as u32, min_x, max_x, min_y, max_y)
    //     .lineplot(&Shape::Lines(&xys))
    //     .nice();

    // wait for the reader to finish
    reader_jh.join().unwrap();

    Ok(())
}
