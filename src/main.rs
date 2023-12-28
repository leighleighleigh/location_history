use anyhow::Result;
use chrono::{DateTime, FixedOffset, Local, NaiveDate, TimeZone};
use geo::Coord;
use spinner::SpinnerBuilder;
use std::path::PathBuf;
use std::sync::mpsc::channel;
use std::thread;

extern crate rerun;
use gnuplot::{Caption, Color, Figure};
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
    #[arg(short = 'n')]
    record_limit: Option<usize>,
    records_json_path: PathBuf,
}

fn main() -> Result<()> {
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
    println!("Removed {} outliers by velocity", delta);

    len_before = filtered_locations.len();

    // filter by activity, start and end date
    if let Some(activity_type) = args.activity_type {
        filtered_locations = filtered_locations.filter_by_activity(activity_type.into());
        // store length after filtering
        println!(
            "Removed {} locations by activity type",
            len_before - filtered_locations.len()
        );
    }

    // print some locations
    for loc in filtered_locations.iter().take(1) {
        println!("{}", loc);
    }

    // make a linestring from the first 10
    // let times: Vec<(i64,Coord<f32>)> = filtered_locations
    //     .iter()
    //     .map(|loc| (loc.timestamp.timestamp_millis(), loc.into()))
    //     .collect();

    let positions: Vec<Coord<f32>> = filtered_locations
        .iter()
        .map(|loc| loc.into())
        .collect();

    // // convert the times to unix epoch seconds/milliseconds
    // let times: Vec<i64> = times.iter().map(|dt| dt.timestamp_millis()).collect();
    // // convert the positions to tuples
    let xys: Vec<(f32, f32)> = positions.iter().map(|p| (p.x, p.y)).collect();
    let xs: Vec<f32> = positions.iter().map(|p| (p.x)).collect();
    let ys: Vec<f32> = positions.iter().map(|p| (p.y)).collect();

    // get min/max of x and y
    let min_x = xs.clone().into_iter().reduce(f32::min).unwrap_or(0.0);
    let max_x = xs.clone().into_iter().reduce(f32::max).unwrap_or(0.0);
    let min_y = ys.clone().into_iter().reduce(f32::min).unwrap_or(0.0);
    let max_y = ys.clone().into_iter().reduce(f32::max).unwrap_or(0.0);
    // use term_size to get the terminal size
    let (w, h) = term_size::dimensions().unwrap();
    Chart::new_with_y_range((w*1) as u32, (h*3) as u32, min_x, max_x, min_y, max_y)
        .lineplot(&Shape::Lines(&xys))
        .nice();

    // // plot with rerun!
    // // Stream log data to an awaiting `rerun` process.
    // let rec = rerun::RecordingStreamBuilder::new("rerun_example_app").connect()?;
    // // let points: Vec<rerun::Position2D> = xys.iter().map(|(x, y)| rerun::Position2D::new(*x, *y)).collect();
    // rec.set_time_sequence("time", 42);
    // // rec.log("position", &rerun::Points2D::new(points)).unwrap();
    // rec.log("position", &rerun::Points2D::new(xys)).unwrap();




    // wait for the reader to finish
    reader_jh.join().unwrap();

    Ok(())
}
