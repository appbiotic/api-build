include!(concat!(
    env!("OUT_DIR"),
    "/appbiotic_api_prost_serde_build/_index.rs"
));

#[cfg(test)]
mod test {
    use prost_wkt_types::{Any, MessageSerde};
    use serde_json::json;

    use crate::prost_serde::{Container, ContainerConfig};

    #[test]
    fn it_compiles() {
        let value = json!({
            "id": 123,
            "ints": [123, 456, 789],
            "config": {
                "@type": "type.googleapis.com/appbiotic.prost_serde_build.examples.ContainerConfig",
                "name": "abc-123"
            }
        });
        let container: Container = serde_json::from_value(value).unwrap();
        let config_any: Any = container.config.unwrap();
        let config_unpacked: Box<dyn MessageSerde> = config_any.try_unpack().unwrap();
        let config_ref: &ContainerConfig =
            config_unpacked.downcast_ref::<ContainerConfig>().unwrap();

        assert_eq!(config_ref.name(), "abc-123");
    }
}
