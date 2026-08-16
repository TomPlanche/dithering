//! HTTP backend over the core dithering pipeline. No routes yet.

/// Placeholder until the routes land. Proves the workspace dependency is wired.
pub fn hello() -> String {
    let (width, height) = dithering_core::DEFAULT_SIZE;

    format!("Hello, world! from dithering-server, core dithers at {width}x{height}")
}

#[cfg(test)]
mod tests {
    use super::hello;

    #[test]
    fn hello_reaches_the_core() {
        assert!(hello().contains("600x400"));
    }
}
