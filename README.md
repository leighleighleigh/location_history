# Use Google location history files in rust
## Preface

This is a personal fork, modified to work with the latest location history format.

Overall the crate isn't too complicated - `serde_json` does all the heavy lifting.

I'm experimenting with channel-based 'streaming deserialization', and parsing of the `activity` field for filtering by vehicle,walking,running, etc.

## [NEW] CLI tool

Feel free to hack on the `main.rs` file to suit your needs - here's the current behaviour.

`$ cargo run --release -- load -h`
```
Google Takeout Records.json Parser

Usage: location-history load [OPTIONS] <RECORDS_JSON_PATH>

Arguments:
  <RECORDS_JSON_PATH>  

Options:
  -s <START_DATE>         
  -e <END_DATE>           
  -a <ACTIVITY_TYPE>      
  -n <RECORD_LIMIT>       
  -h, --help              Print help
  -V, --version           Print version
```

`$ time cargo run --release -- load -s 16_07_14 -e 16_12_31 -a ON_FOOT ./Records.json`
```
Loading data...
55245 returned, 1403330 parsed
Removed 402 outliers by velocity
Removed 53320 locations by activity type
+------------+--------------------------------+
| timestamp  | 2016-07-14 21:22:16.132 +00:00 |
+------------+--------------------------------+
| latitude   | -37.69557                      |
+------------+--------------------------------+
| longitude  | 142.3614                       |
+------------+--------------------------------+
| accuracy   | 24                             |
+------------+--------------------------------+
| altitude   | ???                            |
+------------+--------------------------------+
| activities | 2016-07-14 21:22:13.336 +00:00 |
|            | ON_FOOT         ( 60%)         |
|            | WALKING         ( 58%)         |
|            | UNKNOWN         ( 15%)         |
|            | ON_BICYCLE      ( 13%)         |
|            | IN_VEHICLE      ( 10%)         |
|            | STILL           (  2%)         |
|            | RUNNING         (  2%)         |
+------------+--------------------------------+

```

## Getting started (original)

```rust
extern crate location_history;

use location_history::LocationsExt;

let mut contents = String::new();
File::open(file).unwrap().read_to_string(&mut contents).unwrap();
let locations = location_history.deserialize(&contents).filter_outliers();
for location in locations {
    println!("{:?}", location);
}
```
