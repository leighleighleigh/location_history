default:
    cargo run --release -- load ~/Downloads/takeout-20240315T103310Z-001/Takeout/Location\ History\ \(Timeline\)/Records.json -c -37.82240629213988 145.07038990361727 250 -s 24_01_01 -e 24_03_31
quick:
    cargo run --release -- load ~/Downloads/takeout-20240315T103310Z-001/Takeout/Location\ History\ \(Timeline\)/Records.json -n 10000
