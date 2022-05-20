use crate::config;
use crate::constants;
use crate::exporter;
use crate::http;
use crate::license;

use chrono::NaiveDateTime;
use lazy_static::lazy_static;
use log::{debug, error, warn};
use prometheus::{GaugeVec, IntGaugeVec, Opts};
use regex::Regex;
use serde::Deserialize;
use simple_error::bail;
use std::collections::HashMap;
use std::error::Error;

lazy_static! {
    pub static ref HASP_FEATURES_TOTAL: IntGaugeVec = IntGaugeVec::new(
        Opts::new("hasp_feature_issued", "Total number of issued licenses"),
        &["app", "name"],
    )
    .unwrap();
    pub static ref HASP_FEATURES_USED: IntGaugeVec = IntGaugeVec::new(
        Opts::new("hasp_feature_used", "Number of used licenses"),
        &["app", "name"],
    )
    .unwrap();
    pub static ref HASP_FEATURE_EXPIRATION: GaugeVec = GaugeVec::new(
        Opts::new(
            "hasp_feature_expiration_seconds",
            "Time until license features will expire"
        ),
        &["app", "index", "licenses", "name"]
    )
    .unwrap();
    pub static ref HASP_FEATURE_AGGREGATED_EXPIRATION: GaugeVec = GaugeVec::new(
        Opts::new(
            "hasp_feature_aggregate_expiration_seconds",
            "Aggregated licenses by expiration time"
        ),
        &["app", "features", "index", "licenses"]
    )
    .unwrap();
}

#[derive(Deserialize)]
struct HaspFeature {
    pub fid: Option<String>,
    #[serde(rename = "fn")]
    pub fname: Option<String>,
    pub lic: Option<String>,
    pub logc: Option<String>,
    pub logl: Option<String>,
}

struct HaspExpiration {
    pub feature: String,
    pub expiration: f64,
    pub license_count: i64,
}

pub fn fetch(lic: &config::Hasp) -> Result<(), Box<dyn Error>> {
    lazy_static! {
        static ref RE_HASP_EXPIRATION: Regex =
            Regex::new(r"^.*(\w{3} \w{3} \d+, \d+ \d+:\d+).*$").unwrap();
    }

    let mut http_client = http::build_client(false, "", constants::DEFAULT_TIMEOUT)?;
    let mut expiring = Vec::<HaspExpiration>::new();
    let mut aggregated_expiration: HashMap<String, Vec<HaspExpiration>> = HashMap::new();
    let mut expiration_dates = Vec::<f64>::new();

    /*
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

    let url = format!(
        "http://{}:{}/_int_/tab_feat.html?haspid={}",
        server, port, lic.hasp_key
    );
    let mut user: &str = "";
    let mut pass: &str = "";
    if let Some(auth) = &lic.authentication {
        user = &auth.username;
        pass = &auth.password;
    }

    let reply = http::get(&mut http_client, &url, user, pass)?;
    let features: Vec<HaspFeature> = match serde_json::from_str(&massage(reply)) {
        Ok(v) => v,
        Err(e) => bail!(
            "Can't decode response for HASP feature information from {} as JSON - {}",
            lic.name,
            e
        ),
    };

    for feat in features {
        if feat.fid.is_some() {
            let fid = match feat.fid {
                Some(v) => v,
                None => {
                    // Can't happen
                    panic!();
                }
            };

            if license::is_excluded(&lic.excluded_features, fid.to_string()) {
                debug!("hasp.rs:fetch: Skipping feature id {} because it is in excluded_features list of {}", fid, lic.name);
                continue;
            }

            let mut fname = match feat.fname {
                Some(v) => v,
                None => fid.clone(),
            };

            if fname.is_empty() {
                fname = fid.clone();
            }

            let _logc = match feat.logc {
                Some(v) => v,
                None => {
                    bail!(
                        "hasp.rs:fetch: BUG - Feature with id {} for {} found without logc field!",
                        fid,
                        lic.name
                    );
                }
            };
            let logc: i64 = match _logc.parse() {
                Ok(v) => v,
                Err(e) => {
                    bail!("Can't convert {} to an integer: {}", _logc, e);
                }
            };

            let _logl = match feat.logl {
                Some(v) => v,
                None => {
                    bail!(
                        "hasp.rs:fetch: BUG - Feature with id {} for {} found without logl field!",
                        fid,
                        lic.name
                    );
                }
            };
            let logl: i64 = match _logl.parse() {
                Ok(v) => v,
                Err(e) => {
                    bail!("Can't convert {} to an integer: {}", _logl, e);
                }
            };

            debug!(
                "hasp.rs:fetch: Setting hasp_feature_issued {} {} -> {}",
                lic.name, fname, logl
            );
            HASP_FEATURES_TOTAL
                .with_label_values(&[&lic.name, &fname])
                .set(logl);

            debug!(
                "hasp.rs:fetch: Setting hasp_feature_used {} {} -> {}",
                lic.name, fname, logc
            );
            HASP_FEATURES_USED
                .with_label_values(&[&lic.name, &fname])
                .set(logc);

            let _licexp = match feat.lic {
                Some(v) => v,
                None => {
                    bail!(
                        "hasp.rs:fetch: BUG - Feature with id {} for {} found without lic field!",
                        fid,
                        lic.name
                    );
                }
            };
            if _licexp.is_empty() {
                warn!(
                    "Skippig license expiration for feature id {} of {} because lic field is empty",
                    fid, lic.name
                );
                continue;
            }

            let expiration: f64;
            if _licexp == "Perpetual" {
                expiration = f64::INFINITY
            } else if let Some(capt) = RE_HASP_EXPIRATION.captures(&_licexp) {
                let _expiration = capt.get(1).map_or("", |m| m.as_str());

                if _expiration.is_empty() {
                    bail!(
                        "BUG - can't parse HASP license expiration from {} of feature id {} for {}",
                        _licexp,
                        fid,
                        lic.name
                    );
                }

                expiration = match NaiveDateTime::parse_from_str(_expiration, "%a %b %d, %Y %H:%M")
                {
                    Ok(v) => v.timestamp() as f64,
                    Err(e) => {
                        error!("Can't parse {} as date and time: {}", _expiration, e);
                        continue;
                    }
                }
            } else {
                bail!(
                    "BUG - can't parse HASP license expiration from {} of feature id {} for {}",
                    _licexp,
                    fid,
                    lic.name
                );
            }

            expiration_dates.push(expiration);
            expiring.push(HaspExpiration {
                feature: fname.clone(),
                license_count: logl,
                expiration,
            });

            let expiration_str = expiration.to_string();
            let aggregated = aggregated_expiration
                .entry(expiration_str)
                .or_insert_with(Vec::<HaspExpiration>::new);
            aggregated.push(HaspExpiration {
                feature: fname,
                license_count: logl,
                expiration,
            });
        }
    }

    let mut index: i64 = 1;
    for entry in expiring {
        if license::is_excluded(&lic.excluded_features, entry.feature.to_string()) {
            debug!(
                "hasp.rs:fetch: Skipping feature {} because it is in excluded_features list of {}",
                entry.feature, lic.name
            );
            continue;
        }
        debug!(
            "hasp.rs:fetch: Setting hasp_feature_used_users {} {} {} {} -> {}",
            lic.name,
            index,
            entry.license_count.to_string(),
            entry.feature,
            entry.expiration
        );
        HASP_FEATURE_EXPIRATION
            .with_label_values(&[
                &lic.name,
                &index.to_string(),
                &entry.license_count.to_string(),
                &entry.feature,
            ])
            .set(entry.expiration);
        index += 1;
    }

    index = 0;

    expiration_dates.sort_by(|a, b| a.partial_cmp(b).unwrap());
    expiration_dates.dedup_by(|a, b| a == b);

    for exp in expiration_dates {
        let exp_str = exp.to_string();
        if let Some(v) = aggregated_expiration.get(&exp_str) {
            let mut license_count: i64 = 0;
            let mut feature_count: i64 = 0;
            for entry in v {
                license_count += entry.license_count;
                feature_count += 1;
            }
            debug!("hasp.rs:fetch: Setting hasp_feature_aggregate_expiration_seconds {} {} {} {} -> {}", lic.name, feature_count, index, license_count, exp);
            HASP_FEATURE_AGGREGATED_EXPIRATION
                .with_label_values(&[
                    &lic.name,
                    &feature_count.to_string(),
                    &index.to_string(),
                    &license_count.to_string(),
                ])
                .set(exp);
            index += 1;
        } else {
            warn!("Key {} not found in HashMap aggregated", exp_str);
        }
    }

    Ok(())
}

fn massage(b0rken: String) -> String {
    lazy_static! {
        static ref RE_C_STYLE_COMMENT: Regex = Regex::new(r"/\*.*?\*/").unwrap();
    }
    // HASP data is in JSON format but it includes C-style  comments (/* ... */) and it lacks
    // the JSON notation for an array
    let massaged = b0rken.replace('\r', "").replace('\n', "");
    format!("[ {} ]", RE_C_STYLE_COMMENT.replace_all(&massaged, ""))
}

pub fn register() {
    exporter::REGISTRY
        .register(Box::new(HASP_FEATURES_TOTAL.clone()))
        .unwrap();
    exporter::REGISTRY
        .register(Box::new(HASP_FEATURES_USED.clone()))
        .unwrap();
    exporter::REGISTRY
        .register(Box::new(HASP_FEATURE_EXPIRATION.clone()))
        .unwrap();
    exporter::REGISTRY
        .register(Box::new(HASP_FEATURE_AGGREGATED_EXPIRATION.clone()))
        .unwrap();
}
