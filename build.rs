use std::env;
use std::fs;
use std::path::Path;
use chrono::Local;

fn main() {
    println!("cargo:warning=Build script is running...");

    // Increment the build number
    let out_dir = env::var_os("OUT_DIR").unwrap();
    let build_number_path = Path::new(&out_dir).join("build_number");
    
    let build_number = if let Ok(contents) = fs::read_to_string(&build_number_path) {
        let num = contents.parse::<u64>().unwrap_or(0) + 1;
        println!("cargo:warning=Incrementing build number to {}", num);
        num
    } else {
        println!("cargo:warning=Starting with build number 1");
        1
    };
    
    fs::write(&build_number_path, build_number.to_string()).unwrap();
    
    // Create the version string with timestamp
    let timestamp = Local::now().format("%Y%m%d_%H%M%S");
    let version = format!("{}.{}", build_number, timestamp);
    println!("cargo:warning=Setting BUILD_VERSION={}", version);
    println!("cargo:rustc-env=BUILD_VERSION={}", version);
    
    // Force rebuild when source changes
    println!("cargo:rerun-if-changed=src/main.rs");
    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rerun-if-changed=src/db.rs");
    println!("cargo:rerun-if-changed=build.rs");
} 