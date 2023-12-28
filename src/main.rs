use std::path::PathBuf;
use std::sync::mpsc::channel;
use std::thread;
use anyhow::Result;
use spinner::SpinnerBuilder;

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
    start_date : Option<String>,
    #[arg(short = 'e')]
    end_date : Option<String>,
    #[arg(short = 'a')]
    activity_type : Option<String>,
    records_json_path: PathBuf,
}

fn main() -> Result<()> {
    let LocationHistoryCLI::Load(args) = LocationHistoryCLI::parse();

    // a background thread performs streaming deserialization, while the main thread
    // handles the Locations as they are deserialized. filtering is performed in the main thread.
    let (tx, rx) = channel();
    let mut locations : Vec<Location> = Vec::new();

    // spawn a thread to read the json file 'in the background'
    let reader_jh = thread::spawn(move|| {
        location_history::deserialize_streaming(args.records_json_path, tx);
    });


    // main thread handles the Locations as they are deserialized
    let sp = SpinnerBuilder::new("Loading...".into()).start();

    for loc in rx {
        // put into buffer
        locations.push(loc);

        sp.update(format!("Loading... ({} locations)", locations.len()));
    }
    
    sp.message(format!("{} Locations loaded.", locations.len()));
    sp.close();

    let filtered_locations = locations.filter_outliers().filter_by_activity("ON_FOOT".into());

    // wait for the reader to finish
    reader_jh.join().unwrap();

    Ok(())
}

