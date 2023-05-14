use {
    web3::types::U256,
    serde::{de, Deserializer, Serializer},
    serde_with::{DeserializeAs, SerializeAs},
    std::fmt,
};

pub struct DecimalU256;

impl<'de> DeserializeAs<'de, U256> for DecimalU256 {
    fn deserialize_as<D>(deserializer: D) -> Result<U256, D::Error>
        where
            D: Deserializer<'de>,
    {
        deserialize(deserializer)
    }
}

impl SerializeAs<U256> for DecimalU256 {
    fn serialize_as<S>(source: &U256, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
    {
        serialize(source, serializer)
    }
}

pub fn serialize<S>(value: &U256, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
{
    serializer.serialize_str(&value.to_string())
}

pub fn deserialize<'de, D>(deserializer: D) -> Result<U256, D::Error>
    where
        D: Deserializer<'de>,
{
    struct Visitor {}
    impl<'de> de::Visitor<'de> for Visitor {
        type Value = U256;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            write!(formatter, "a u256 encoded as a decimal encoded string")
        }

        fn visit_str<E>(self, s: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
        {
            U256::from_dec_str(s).map_err(|err| {
                de::Error::custom(format!("failed to decode {s:?} as decimal u256: {err}"))
            })
        }
    }

    deserializer.deserialize_str(Visitor {})
}
