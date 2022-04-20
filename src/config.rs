use serde::Deserialize;
use std::error::Error;
use std::fs;

#[derive(Clone, Debug, Deserialize)]
pub struct Configuration {
    pub global: Option<GlobalConfiguration>,
    pub flexlm: Option<Vec<FlexLM>>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GlobalConfiguration {
    pub lmstat: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct FlexLM {
    pub name: String,
    pub license: String,
}

pub fn parse_config_file(f: &str) -> Result<Configuration, Box<dyn Error>> {
    let unparsed = fs::read_to_string(f)?;
    let mut config: Configuration = serde_yaml::from_str(unparsed.as_str())?;

    validate_configuration(&config)?;

    Ok(config)
}

fn validate_configuration(cfg: &Configuration) -> Result<(), Box<dyn Error>> {
    Ok(())
}
