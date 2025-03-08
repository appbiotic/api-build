#[macro_export]
macro_rules! bindings {
    () => {
        #[cfg(feature = "prost-serde")]
        pub mod prost_serde {
            include!(concat!(
                env!("OUT_DIR"),
                "/appbiotic_api_prost_serde_build/_index.rs"
            ));
        }
    };
}
