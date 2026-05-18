//! This build script copies the `memory.x` file from the crate root into
//! a directory where the linker can always find it at build time.
//! For many projects this is optional, as the linker always searches the
//! project root directory -- wherever `Cargo.toml` is. However, if you
//! are using a workspace or have a more complicated build setup, this
//! build script becomes required. Additionally, by requesting that
//! Cargo re-run the build script whenever `memory.x` is changed,
//! updating `memory.x` ensures a rebuild of the application with the
//! new memory settings.

use build_log as log;
use build_script_directives as cargo;
use memory_x_tools::copy_memory_x;
use std::env;

fn forward_env_var() {
    if let Err(err) = dotenvy::dotenv() {
        log::warning!("Failed to load .env file: {}", err);
        return;
    }

    let forward_list = ["WIFI_SSID", "WIFI_PASSWORD"];

    for var_name in forward_list {
        if let Ok(var_value) = env::var(var_name) {
            println!("cargo:rustc-env={}={}", var_name, var_value);
        }
    }
}

fn main() {
    /**************************************************************************************
     *  Bypass the WiFi password and WiFi SSID in plaintext to the Rust code from the
     * .env file.
     **************************************************************************************/
    forward_env_var();
    // Rebuild if .env file changes
    cargo::cmd!("rerun-if-changed=.env");

    /**************************************************************************************
     *  Linker configuration
     **************************************************************************************/

    // Put `memory.x` in our output directory and ensure it's
    // on the linker search path.
    copy_memory_x().expect("Failed to copy memory.x");

    // Specify link flags for the linker script and other settings.
    cargo::cmd!("rustc-link-arg-bins=--nmagic");
    cargo::cmd!("rustc-link-arg-bins=-Tlink.x");
    cargo::cmd!("rustc-link-arg-bins=-Tlink-rp.x");
    cargo::cmd!("rustc-link-arg-bins=-Tdefmt.x");
}
