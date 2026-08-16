//! HTTP backend over the core dithering pipeline. No routes yet.

/// Placeholder until the routes land. Proves the workspace dependency is wired.
pub fn hello() -> String {
    format!(
        "Hello, world! from dithering-server, core says: {}",
        dithering_core::hello()
    )
}

#[cfg(test)]
mod tests {
    use super::hello;

    #[test]
    fn hello_reaches_the_core() {
        assert!(hello().contains("dithering-core"));
    }
}
