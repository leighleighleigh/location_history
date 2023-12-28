# Use Google location history files in rust
## Preface

This is a personal fork, modified to work with the latest location history format.

Overall the crate isn't too complicated - `serde_json` does all the heavy lifting.

I'm experimenting with channel-based 'streaming deserialization', and parsing of the `activity` field for filtering by vehicle,walking,running, etc.

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
