use serde::Deserialize;
use simple_error::bail;
use std::error::Error;
use std::fs;

#[derive(Clone, Debug, Deserialize)]
pub struct Configuration {
    pub global: Option<GlobalConfiguration>,
    pub flexlm: Option<Vec<FlexLM>>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GlobalConfiguration {
    pub lmutil: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct FlexLM {
    pub name: String,
    pub license: String,
    pub excluded_features: Option<Vec<String>>,
    pub export_user: Option<bool>,
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
                bail!("Empty name for FlexLM license {}", flex.name);
            }

            if flex.license.is_empty() {
                bail!(
                    "Missing license information for FlexLM license {}",
                    flex.name
                );
            }
        }
    }
    Ok(())
}
