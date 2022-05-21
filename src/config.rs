use serde::Deserialize;
use simple_error::bail;
use std::error::Error;
use std::fs;

#[derive(Clone, Debug, Deserialize)]
pub struct Configuration {
    pub dsls: Option<Vec<Dsls>>,
    pub flexlm: Option<Vec<FlexLM>>,
    pub global: Option<GlobalConfiguration>,
    pub hasp: Option<Vec<Hasp>>,
    pub licman20: Option<Vec<Licman20>>,
    pub lmx: Option<Vec<Lmx>>,
    pub rlm: Option<Vec<Rlm>>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GlobalConfiguration {
    pub dslicsrv: Option<String>,
    pub licman20_appl: Option<String>,
    pub lmutil: Option<String>,
    pub lmxendutil: Option<String>,
    pub rlmutil: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Dsls {
    pub excluded_features: Option<Vec<String>>,
    pub export_user: Option<bool>,
    pub license: String,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct FlexLM {
    pub excluded_features: Option<Vec<String>>,
    pub export_user: Option<bool>,
    pub license: String,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Rlm {
    pub excluded_features: Option<Vec<String>>,
    pub export_user: Option<bool>,
    pub isv: String,
    pub license: String,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Lmx {
    pub excluded_features: Option<Vec<String>>,
    pub export_user: Option<bool>,
    pub license: String,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Licman20 {
    pub excluded_features: Option<Vec<String>>,
    pub export_user: Option<bool>,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Hasp {
    pub authentication: Option<HaspAuth>,
    pub excluded_features: Option<Vec<String>>,
    pub export_user: Option<bool>,
    pub hasp_key: String,
    pub license: String,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct HaspAuth {
    pub username: String,
    pub password: String,
}

pub fn parse_config_file(f: &str) -> Result<Configuration, Box<dyn Error>> {
    let unparsed = fs::read_to_string(f)?;
    let config: Configuration = serde_yaml::from_str(unparsed.as_str())?;

    validate_configuration(&config)?;

    Ok(config)
}

fn validate_configuration(cfg: &Configuration) -> Result<(), Box<dyn Error>> {
    if let Some(flexlm) = &cfg.flexlm {
        for flex in flexlm {
            if flex.name.is_empty() {
                bail!("Empty name for FlexLM license");
            }

            if flex.license.is_empty() {
                bail!(
                    "Missing license information for FlexLM license {}",
                    flex.name
                );
            }
        }
    }

    if let Some(rlm) = &cfg.rlm {
        for _rlm in rlm {
            if _rlm.name.is_empty() {
                bail!("Empty name for RLM license");
            }

            if _rlm.license.is_empty() {
                bail!("Missing license information for RLM license {}", _rlm.name);
            }
            if _rlm.isv.is_empty() {
                bail!("Missing ISV for RLM license {}", _rlm.name);
            }
        }
    }

    if let Some(lmx) = &cfg.lmx {
        for _lmx in lmx {
            if _lmx.name.is_empty() {
                bail!("Empty name for LM-X license");
            }

            if _lmx.license.is_empty() {
                bail!("Missing license information for LM-X license {}", _lmx.name);
            }

            for lsrv in _lmx.license.split(':') {
                if lsrv.contains('@') && lsrv.split('@').count() != 2 {
                    bail!("Invalid license for LM-X license {}", _lmx.name);
                }
            }

            if _lmx.license.contains(':') {
                let srvcnt: Vec<&str> = _lmx.license.split(':').collect();
                if srvcnt.len() != 3 {
                    bail!("Only three servers are allowed for LM-X HAL servers instead of {} for license {}", srvcnt.len(), _lmx.name);
                }
            }
        }
    }

    if let Some(dsls) = &cfg.dsls {
        for _dsls in dsls {
            if _dsls.name.is_empty() {
                bail!("Empty name for DSLS license");
            }

            if _dsls.license.is_empty() {
                bail!(
                    "Missing license information for DSLS license {}",
                    _dsls.name
                );
            }

            for lsrv in _dsls.license.split(':') {
                if !lsrv.contains('@') {
                    bail!("Invalid license for DSLS license {}", _dsls.name);
                }
            }

            if _dsls.license.contains(':') {
                let srvcnt: Vec<&str> = _dsls.license.split(':').collect();
                if srvcnt.len() != 3 {
                    bail!("Only three servers are allowed for redundant DSLS servers instead of {} for license {}", srvcnt.len(), _dsls.name);
                }
            }
        }
    }

    if let Some(hasp) = &cfg.hasp {
        for _hasp in hasp {
            if _hasp.name.is_empty() {
                bail!("Empty name for HASP license");
            }

            if _hasp.license.is_empty() {
                bail!(
                    "Missing license information for HASP license {}",
                    _hasp.name
                );
            }

            if let Some(auth) = &_hasp.authentication {
                if auth.username.is_empty() {
                    bail!(
                        "HASP authentication requires a username for HASP license {}",
                        _hasp.name
                    );
                }
                if auth.password.is_empty() {
                    bail!(
                        "HASP authentication require a password for HASP license {}",
                        _hasp.name
                    );
                }
            }
        }
    }

    Ok(())
}
