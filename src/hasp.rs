use crate::config;
use crate::constants;
use crate::http;

/*
use crate::exporter;
use crate::license;

use chrono::NaiveDateTime;
use lazy_static::lazy_static;
use log::{debug, error, warn};
use prometheus::{GaugeVec, IntGaugeVec, Opts};
use regex::Regex;
use simple_error::bail;
use std::collections::HashMap;
use std::env;
*/
use std::error::Error;
/*
use std::process::Command;

lazy_static! {}
*/
pub fn fetch(lic: &config::Hasp) -> Result<(), Box<dyn Error>> {
    let http_client = http::build_client(false, "", constants::DEFAULT_TIMEOUT)?;
    /*
        lazy_static! {
            static ref RE_DSLS_VERSION: Regex =
                Regex::new(r"^\s+Software version:\s+([\d.\-]+)$").unwrap();
            static ref RE_DSLS_STATUS: Regex = Regex::new(r"^\s+Ready:\s+(\w+).*$").unwrap();
        }

        // dict -> "feature" -> "user" -> count
        let mut fuv: HashMap<String, HashMap<String, i64>> = HashMap::new();
    */

    let server: &str;
    let mut port: &str = constants::DEFAULT_HASP_PORT;
    if lic.license.contains('@') {
        let splitted: Vec<&str> = lic.license.split('@').collect();
        port = splitted[0];
        server = splitted[1];
    } else {
        server = &lic.license;
    }

    Ok(())
}

pub fn register() {}
