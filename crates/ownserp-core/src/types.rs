use std::num::NonZeroU32;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct CreditCost(u32);

impl CreditCost {
    pub const fn new(credits: u32) -> Self {
        Self(credits)
    }

    pub const fn get(self) -> u32 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QpsLimit(NonZeroU32);

impl QpsLimit {
    pub fn new(raw: u32) -> Result<Self, Error> {
        NonZeroU32::new(raw)
            .map(Self)
            .ok_or_else(|| Error::Config("queries per second must be at least 1".into()))
    }

    pub const fn get(self) -> NonZeroU32 {
        self.0
    }
}

impl Default for QpsLimit {
    fn default() -> Self {
        Self(NonZeroU32::MIN)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "String", into = "String")]
pub struct MapViewport(String);

impl MapViewport {
    pub fn as_query(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for MapViewport {
    type Error = Error;

    fn try_from(raw: String) -> Result<Self, Self::Error> {
        let trimmed = raw.trim();
        if trimmed.len() > 64 {
            return Err(Error::InvalidArgs(
                "ll is longer than 64 bytes; it takes latitude,longitude,zoom".into(),
            ));
        }
        let mut parts = trimmed.split(',');
        let lat = parts
            .next()
            .ok_or_else(|| Error::InvalidArgs("ll needs latitude,longitude,zoom".into()))?
            .trim_start_matches('@')
            .trim()
            .parse::<f64>()
            .map_err(|_| Error::InvalidArgs(format!("ll latitude is not a number: {raw}")))?;
        let lng = parts
            .next()
            .ok_or_else(|| Error::InvalidArgs("ll needs latitude,longitude,zoom".into()))?
            .trim()
            .parse::<f64>()
            .map_err(|_| Error::InvalidArgs(format!("ll longitude is not a number: {raw}")))?;
        let zoom_raw = parts
            .next()
            .ok_or_else(|| Error::InvalidArgs("ll needs latitude,longitude,zoom".into()))?;
        if parts.next().is_some() {
            return Err(Error::InvalidArgs(format!(
                "ll takes exactly three parts, got: {raw}"
            )));
        }
        let zoom_text = zoom_raw.trim();
        let zoom_digits = zoom_text
            .strip_suffix('z')
            .ok_or_else(|| Error::InvalidArgs(format!("ll zoom must end with z: {raw}")))?;
        let zoom = zoom_digits
            .parse::<u8>()
            .map_err(|_| Error::InvalidArgs(format!("ll zoom is not a number: {raw}")))?;
        if !(-90.0..=90.0).contains(&lat) {
            return Err(Error::InvalidArgs(format!(
                "latitude {lat} is outside -90..90"
            )));
        }
        if !(-180.0..=180.0).contains(&lng) {
            return Err(Error::InvalidArgs(format!(
                "longitude {lng} is outside -180..180"
            )));
        }
        if !(1..=21).contains(&zoom) {
            return Err(Error::InvalidArgs(format!("zoom {zoom} is outside 1..21")));
        }
        Ok(Self(format!("@{lat},{lng},{zoom}z")))
    }
}

impl From<MapViewport> for String {
    fn from(value: MapViewport) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "String", into = "String")]
pub struct LocationQuery(String);

impl LocationQuery {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for LocationQuery {
    type Error = Error;

    fn try_from(raw: String) -> Result<Self, Self::Error> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(Error::InvalidArgs("location is empty".into()));
        }
        if trimmed.chars().count() > 256 {
            return Err(Error::InvalidArgs(
                "location is longer than 256 characters".into(),
            ));
        }
        if trimmed.chars().any(char::is_control) {
            return Err(Error::InvalidArgs(
                "location contains control characters".into(),
            ));
        }
        Ok(Self(trimmed.to_owned()))
    }
}

impl From<LocationQuery> for String {
    fn from(value: LocationQuery) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum TimeFilter {
    #[serde(rename = "qdr:h")]
    Hour,
    #[serde(rename = "qdr:d")]
    Day,
    #[serde(rename = "qdr:w")]
    Week,
    #[serde(rename = "qdr:m")]
    Month,
    #[serde(rename = "qdr:y")]
    Year,
}

impl TimeFilter {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Hour => "qdr:h",
            Self::Day => "qdr:d",
            Self::Week => "qdr:w",
            Self::Month => "qdr:m",
            Self::Year => "qdr:y",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "lowercase")]
pub enum Preset {
    Minimal,
    #[default]
    Standard,
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "lowercase")]
pub enum TextMode {
    #[default]
    Summary,
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "lowercase")]
pub enum PackTier {
    #[default]
    Starter,
    Standard,
    Scale,
    Ultimate,
    Custom,
}

impl PackTier {
    pub const fn usd_per_thousand(self) -> Option<f64> {
        match self {
            Self::Starter => Some(1.00),
            Self::Standard => Some(0.75),
            Self::Scale => Some(0.50),
            Self::Ultimate => Some(0.30),
            Self::Custom => None,
        }
    }

    pub const fn qps(self) -> u32 {
        match self {
            Self::Starter => 50,
            Self::Standard => 100,
            Self::Scale => 200,
            Self::Ultimate => 300,
            Self::Custom => 50,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiKeySource {
    Environment,
    Keychain,
    Missing,
}

impl ApiKeySource {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Environment => "environment",
            Self::Keychain => "keychain",
            Self::Missing => "missing",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewport_round_trips_the_live_form() {
        let viewport = MapViewport::try_from("@40.7504178,-73.9824837,14z".to_owned())
            .expect("valid viewport");
        assert_eq!(viewport.as_query(), "@40.7504178,-73.9824837,14z");
    }

    #[test]
    fn viewport_rejects_bad_input() {
        for raw in [
            "40.7,-73.9",
            "@200,-73.9,14z",
            "@40.7,-200,14z",
            "@40.7,-73.9,0z",
            "@40.7,-73.9,14",
            "@40.7,-73.9,14z,extra",
        ] {
            assert!(
                MapViewport::try_from(raw.to_owned()).is_err(),
                "{raw} should not parse"
            );
        }
    }

    #[test]
    fn time_filter_uses_the_live_query_value() {
        assert_eq!(
            serde_json::to_string(&TimeFilter::Week).expect("serialize"),
            "\"qdr:w\""
        );
    }

    #[test]
    fn pack_rates_match_the_published_table() {
        assert_eq!(PackTier::Starter.usd_per_thousand(), Some(1.0));
        assert_eq!(PackTier::Ultimate.usd_per_thousand(), Some(0.3));
        assert_eq!(PackTier::Custom.usd_per_thousand(), None);
        assert_eq!(PackTier::Scale.qps(), 200);
    }

    #[test]
    fn location_rejects_control_characters() {
        assert!(LocationQuery::try_from("Dhaka, Bangladesh".to_owned()).is_ok());
        assert!(LocationQuery::try_from("   ".to_owned()).is_err());
        assert!(LocationQuery::try_from("Dhaka\u{7}".to_owned()).is_err());
    }
}
