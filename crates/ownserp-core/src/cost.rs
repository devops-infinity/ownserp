use crate::types::{CreditCost, PackTier};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Host {
    Google,
    Scrape,
    Api,
}

impl Host {
    pub const fn base_url(self) -> &'static str {
        match self {
            Self::Google => "https://google.serper.dev",
            Self::Scrape => "https://scrape.serper.dev",
            Self::Api => "https://api.serper.dev",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Endpoint {
    Search,
    Images,
    News,
    Places,
    Maps,
    Videos,
    Shopping,
    Scholar,
    Patents,
    Autocomplete,
    Reviews,
    Lens,
    Scrape,
    Locations,
}

pub const BATCH_MULTIPLIER: u32 = 3;
pub(crate) const SCRAPE_RESERVATION_CREDITS: u32 = 10;

impl Endpoint {
    pub const fn path(self) -> &'static str {
        match self {
            Self::Search => "/search",
            Self::Images => "/images",
            Self::News => "/news",
            Self::Places => "/places",
            Self::Maps => "/maps",
            Self::Videos => "/videos",
            Self::Shopping => "/shopping",
            Self::Scholar => "/scholar",
            Self::Patents => "/patents",
            Self::Autocomplete => "/autocomplete",
            Self::Reviews => "/reviews",
            Self::Lens => "/lens",
            Self::Scrape => "/",
            Self::Locations => "/locations",
        }
    }

    pub const fn host(self) -> Host {
        match self {
            Self::Scrape => Host::Scrape,
            Self::Locations => Host::Api,
            _ => Host::Google,
        }
    }

    pub const fn base_cost(self) -> CreditCost {
        match self {
            Self::Maps | Self::Lens => CreditCost::new(3),
            Self::Shopping => CreditCost::new(2),
            Self::Scrape => CreditCost::new(2),
            Self::Locations => CreditCost::new(0),
            _ => CreditCost::new(1),
        }
    }

    pub const fn tool_name(self) -> &'static str {
        match self {
            Self::Search => "serper_search",
            Self::Images => "serper_images",
            Self::News => "serper_news",
            Self::Places => "serper_places",
            Self::Maps => "serper_maps",
            Self::Videos => "serper_videos",
            Self::Shopping => "serper_shopping",
            Self::Scholar => "serper_scholar",
            Self::Patents => "serper_patents",
            Self::Autocomplete => "serper_autocomplete",
            Self::Reviews => "serper_reviews",
            Self::Lens => "serper_lens",
            Self::Scrape => "serper_scrape",
            Self::Locations => "serper_locations",
        }
    }

    pub const fn requires_key(self) -> bool {
        !matches!(self, Self::Locations)
    }

    pub fn estimated_cost(self, num_results: Option<u8>) -> CreditCost {
        let base = self.base_cost();
        if self == Self::Images && num_results.is_some_and(|count| count > 10) {
            CreditCost::new(base.get().saturating_mul(2))
        } else {
            base
        }
    }
}

pub fn usd_for_call(credits: u32, pack: PackTier, override_rate: Option<f64>) -> Option<f64> {
    let usd_per_1k = override_rate.or_else(|| pack.usd_per_1k())?;
    let raw = f64::from(credits) * usd_per_1k / 1000.0;
    Some(round_micros(raw))
}

pub fn usd_for_credits(credits: u64, pack: PackTier, override_rate: Option<f64>) -> Option<f64> {
    let usd_per_1k = override_rate.or_else(|| pack.usd_per_1k())?;
    let raw = credits as f64 * usd_per_1k / 1000.0;
    Some(round_micros(raw))
}

fn round_micros(dollars: f64) -> f64 {
    (dollars * 1_000_000.0).round() / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credit_table_matches_the_contract() {
        assert_eq!(Endpoint::Search.estimated_cost(None).get(), 1);
        assert_eq!(Endpoint::Maps.estimated_cost(None).get(), 3);
        assert_eq!(Endpoint::Shopping.estimated_cost(None).get(), 2);
        assert_eq!(Endpoint::Lens.estimated_cost(None).get(), 3);
        assert_eq!(Endpoint::Locations.estimated_cost(None).get(), 0);
    }

    #[test]
    fn images_double_past_ten_results() {
        assert_eq!(Endpoint::Images.estimated_cost(Some(10)).get(), 1);
        assert_eq!(Endpoint::Images.estimated_cost(Some(11)).get(), 2);
        assert_eq!(Endpoint::Search.estimated_cost(Some(100)).get(), 1);
    }

    #[test]
    fn dollars_round_to_six_places() {
        assert_eq!(usd_for_call(1, PackTier::Starter, None), Some(0.001));
        assert_eq!(usd_for_call(3, PackTier::Standard, None), Some(0.00225));
        assert_eq!(usd_for_call(1, PackTier::Custom, Some(2.0)), Some(0.002));
    }

    #[test]
    fn an_unset_custom_rate_stays_unknown() {
        assert_eq!(usd_for_call(1, PackTier::Custom, None), None);
        assert_eq!(usd_for_credits(500, PackTier::Custom, None), None);
    }

    #[test]
    fn hosts_split_by_endpoint() {
        assert_eq!(Endpoint::Scrape.host(), Host::Scrape);
        assert_eq!(Endpoint::Locations.host(), Host::Api);
        assert_eq!(Endpoint::Search.host(), Host::Google);
    }
}
