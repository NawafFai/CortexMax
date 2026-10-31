fn main() -> anyhow::Result<()> {
    if std::env::args().nth(1).as_deref() == Some("--native-project-runtime-capabilities") {
        println!("buzz-native-project-harness-v1");
        return Ok(());
    }
    buzz_acp::run()
}
