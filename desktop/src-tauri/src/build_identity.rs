//! Compile-time identity for Cortex Max builds and reusable named demo builds.
//!
//! Production builds leave `BUZZ_DESKTOP_BUILD_DEMO_SLUG` unset and use the
//! Cortex Max identity. The demo recipe validates one slug and `build.rs` bakes
//! it into the binary; every runtime identity is then derived from that value.
//! Every name here is distinct from the upstream Buzz identity (`buzz`,
//! `~/.buzz`, `buzz-desktop`, `xyz.block.buzz.app`) so that a Cortex Max build
//! never reads or writes an installed Buzz.app's state.

use std::borrow::Cow;

pub(crate) fn demo_slug() -> Option<&'static str> {
    option_env!("BUZZ_DESKTOP_BUILD_DEMO_SLUG")
}

pub(crate) fn is_demo_build() -> bool {
    demo_slug().is_some()
}

pub(crate) const DEMO_AGENT_CONFIG_ENV: &str = "BUZZ_AGENT_CONFIG_DIR";

/// Config home handed to bundled child processes through
/// [`DEMO_AGENT_CONFIG_ENV`]. Cortex Max always owns a build-scoped root —
/// `<config_dir>/cortex-max` for production, `<config_dir>/cortex-max-demo-<slug>`
/// for demos — so bundled buzz-agent OAuth tokens never share
/// `~/.config/buzz-agent` with an installed Buzz.app. `None` only when the
/// platform config directory cannot be resolved at all.
pub(crate) fn demo_config_home() -> Result<Option<std::path::PathBuf>, String> {
    demo_config_home_for(demo_slug(), dirs::config_dir())
}

pub(crate) fn demo_agent_oauth_cache_dir() -> Result<Option<std::path::PathBuf>, String> {
    Ok(demo_config_home()?.map(|dir| dir.join("buzz-agent").join("oauth")))
}

/// Keep child config caches inside this build's identity. In particular,
/// bundled buzz-agent OAuth tokens must not read or write another product's
/// root. Refuse launch if a demo cannot resolve its root.
pub(crate) fn apply_demo_config_home(command: &mut std::process::Command) -> Result<(), String> {
    if let Some(config_home) = demo_config_home()? {
        command.env(DEMO_AGENT_CONFIG_ENV, config_home);
    }
    Ok(())
}

fn demo_config_home_for(
    demo_slug: Option<&str>,
    config_dir: Option<std::path::PathBuf>,
) -> Result<Option<std::path::PathBuf>, String> {
    match demo_slug {
        None => Ok(config_dir.map(|dir| dir.join("cortex-max"))),
        Some(slug) => config_dir
            .map(|dir| Some(dir.join(format!("cortex-max-demo-{slug}"))))
            .ok_or_else(|| "cannot resolve demo credential directory".to_string()),
    }
}

pub(crate) fn deep_link_scheme() -> Cow<'static, str> {
    demo_slug()
        .map(|slug| Cow::Owned(format!("cortexmax-demo-{slug}")))
        .unwrap_or(Cow::Borrowed("cortexmax"))
}

pub(crate) fn is_deep_link_for_build(value: &str) -> bool {
    is_deep_link_for_scheme(value, deep_link_scheme().as_ref())
}

fn is_deep_link_for_scheme(value: &str, scheme: &str) -> bool {
    value
        .strip_prefix(scheme)
        .is_some_and(|suffix| suffix.starts_with("://"))
}

pub(crate) fn keyring_service() -> Cow<'static, str> {
    demo_slug()
        .map(|slug| Cow::Owned(format!("cortex-max-desktop-demo.{slug}")))
        .unwrap_or(Cow::Borrowed("cortex-max-desktop"))
}

pub(crate) fn nest_name(is_dev: bool) -> Cow<'static, str> {
    nest_name_for(demo_slug(), is_dev)
}

fn nest_name_for(demo_slug: Option<&str>, is_dev: bool) -> Cow<'_, str> {
    if let Some(slug) = demo_slug {
        Cow::Owned(format!(".cortex-max-demo-{slug}"))
    } else if is_dev {
        Cow::Borrowed(".cortex-max-dev")
    } else {
        Cow::Borrowed(".cortex-max")
    }
}

pub(crate) fn cli_name(is_dev: bool) -> String {
    if let Some(slug) = demo_slug() {
        format!("cortex-max-demo-{slug}")
    } else if is_dev {
        "cortex-max-dev".to_string()
    } else {
        "cortex-max".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "compiled with BUZZ_BUILD_DEMO_SLUG by the compiled-flags recipe"]
    fn compiled_demo_slug_matches_expected() {
        let expected = std::env::var("BUZZ_TEST_EXPECTED_DEMO_SLUG")
            .expect("BUZZ_TEST_EXPECTED_DEMO_SLUG must be set");
        assert_eq!(demo_slug(), Some(expected.as_str()));
    }

    #[test]
    fn ordinary_release_defaults_are_the_cortex_max_identity() {
        if demo_slug().is_none() {
            assert_eq!(deep_link_scheme(), "cortexmax");
            assert_eq!(keyring_service(), "cortex-max-desktop");
            assert_eq!(nest_name(false), ".cortex-max");
            assert_eq!(cli_name(false), "cortex-max");
        }
    }

    #[test]
    fn no_identity_name_collides_with_upstream_buzz() {
        for name in [
            deep_link_scheme().to_string(),
            keyring_service().to_string(),
            nest_name(false).to_string(),
            nest_name(true).to_string(),
            cli_name(false),
            cli_name(true),
        ] {
            assert!(
                ![
                    "buzz",
                    "buzz-dev",
                    ".buzz",
                    ".buzz-dev",
                    "buzz-desktop",
                    "buzz-desktop-dev"
                ]
                .contains(&name.as_str()),
                "{name} would collide with an installed Buzz.app"
            );
        }
    }

    #[test]
    fn agent_config_and_oauth_roots_are_build_scoped() {
        let base = std::path::PathBuf::from("/Users/demo/Library/Application Support");
        let production = demo_config_home_for(None, Some(base.clone()))
            .unwrap()
            .unwrap();
        assert_eq!(
            production,
            std::path::PathBuf::from("/Users/demo/Library/Application Support/cortex-max")
        );
        let first = demo_config_home_for(Some("board-1234567812345678"), Some(base.clone()))
            .unwrap()
            .unwrap();
        let second = demo_config_home_for(Some("board-8765432187654321"), Some(base))
            .unwrap()
            .unwrap();
        assert_eq!(
            first,
            std::path::PathBuf::from(
                "/Users/demo/Library/Application Support/cortex-max-demo-board-1234567812345678"
            )
        );
        assert_eq!(
            first.join("buzz-agent/oauth"),
            std::path::PathBuf::from(
                "/Users/demo/Library/Application Support/cortex-max-demo-board-1234567812345678/buzz-agent/oauth"
            )
        );
        assert_ne!(first, second);
        assert_ne!(first, production);
    }

    #[test]
    fn unresolved_demo_credentials_never_select_production_defaults() {
        assert_eq!(demo_config_home_for(None, None).unwrap(), None);
        assert_eq!(
            demo_config_home_for(Some("board-1234567812345678"), None),
            Err("cannot resolve demo credential directory".to_string())
        );
    }

    #[test]
    fn duplicate_instance_links_follow_the_build_scheme() {
        assert!(is_deep_link_for_scheme(
            "cortexmax://message?id=1",
            "cortexmax"
        ));
        assert!(!is_deep_link_for_scheme("buzz://message?id=1", "cortexmax"));
        assert!(!is_deep_link_for_scheme(
            "cortexmax-demo-board-1234567812345678://message?id=1",
            "cortexmax"
        ));
        assert!(is_deep_link_for_scheme(
            "cortexmax-demo-board-1234567812345678://message?id=1",
            "cortexmax-demo-board-1234567812345678"
        ));
        assert!(!is_deep_link_for_scheme(
            "cortexmax://message?id=1",
            "cortexmax-demo-board-1234567812345678"
        ));
    }

    #[test]
    fn production_and_named_demo_nests_are_distinct() {
        assert_eq!(nest_name_for(None, false), ".cortex-max");
        assert_eq!(nest_name_for(None, true), ".cortex-max-dev");
        assert_eq!(
            nest_name_for(Some("workstream-board"), false),
            ".cortex-max-demo-workstream-board"
        );
        assert_eq!(
            nest_name_for(Some("second-demo"), false),
            ".cortex-max-demo-second-demo"
        );
    }
}
