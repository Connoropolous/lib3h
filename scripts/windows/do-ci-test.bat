@echo off
setlocal enabledelayedexpansion

rem KEEP IN SYNC WITH HOLONIX
set RUST_LOG=lib3h=debug
set RUST_BACKTRACE=1
cargo bench -p lib3h --target-dir c:\build\lib3h\target
cargo test --target-dir c:\build\lib3h\target --verbose
