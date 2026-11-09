use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error};

/// An Audible/Amazon storefront. One marketplace is active at a time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Marketplace {
    /// Short country code used by Audible's iOS sign-in handle, e.g. `us`.
    pub id: &'static str,
    pub name: &'static str,
    /// Top-level domain shared by the Amazon and Audible hosts, e.g. `co.uk`.
    pub domain: &'static str,
    /// Amazon marketplace identifier sent during sign-in.
    pub marketplace_id: &'static str,
}

const fn store(
    id: &'static str,
    name: &'static str,
    domain: &'static str,
    marketplace_id: &'static str,
) -> Marketplace {
    Marketplace {
        id,
        name,
        domain,
        marketplace_id,
    }
}

impl Marketplace {
    pub const US: Self = store("us", "United States", "com", "AF2M0KC94RCEA");
    pub const UK: Self = store("uk", "United Kingdom", "co.uk", "A2I9A3Q2GNFNGQ");
    pub const GERMANY: Self = store("de", "Germany", "de", "AN7V1F1VY261K");
    pub const FRANCE: Self = store("fr", "France", "fr", "A2728XDNODOQ8T");
    pub const ITALY: Self = store("it", "Italy", "it", "A2N7FU2W2BU2ZC");
    pub const SPAIN: Self = store("es", "Spain", "es", "ALMIKO4SZCSAR");
    pub const CANADA: Self = store("ca", "Canada", "ca", "A2CQZ5RBY40XE");
    pub const AUSTRALIA: Self = store("au", "Australia", "com.au", "AN7EY7DTAW63G");
    pub const INDIA: Self = store("in", "India", "in", "AJO3FBRUE6J4S");
    pub const JAPAN: Self = store("jp", "Japan", "co.jp", "A1QAP3MOU4173J");

    /// Supported marketplaces in display order. US is the default.
    pub const ALL: [Self; 10] = [
        Self::US,
        Self::UK,
        Self::GERMANY,
        Self::FRANCE,
        Self::ITALY,
        Self::SPAIN,
        Self::CANADA,
        Self::AUSTRALIA,
        Self::INDIA,
        Self::JAPAN,
    ];

    pub fn with_id(id: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|marketplace| marketplace.id == id)
    }

    /// Host serving the sign-in pages and the login callback.
    pub fn amazon_host(&self) -> String {
        format!("www.amazon.{}", self.domain)
    }

    /// Host serving device registration and token refresh.
    pub fn amazon_api_host(&self) -> String {
        format!("api.amazon.{}", self.domain)
    }

    /// Host serving library and license requests.
    pub fn audible_api_host(&self) -> String {
        format!("api.audible.{}", self.domain)
    }
}

// Persist only the identifier so stored sessions follow corrected marketplace data.
impl Serialize for Marketplace {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.id)
    }
}

impl<'de> Deserialize<'de> for Marketplace {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let id = String::deserialize(deserializer)?;
        Self::with_id(&id).ok_or_else(|| D::Error::custom(format!("Unknown marketplace '{id}'")))
    }
}
