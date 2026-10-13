//! The fork's client-side `[keys]` actions stay local under a host's key
//! profile, also across a live reload.

use super::*;

#[test]
fn live_reload_under_an_endpoint_profile_rereads_only_the_client_actions() {
    let local: Config = toml::from_str(
        r#"
[keys]
prefix = "ctrl+a"
history_back = "alt+left"
"#,
    )
    .unwrap();
    let endpoint: Config = toml::from_str(
        r#"
[keys]
prefix = "ctrl+x"
new_tab = "prefix+n"
"#,
    )
    .unwrap();
    let profile = endpoint.local_keybindings_profile_toml().unwrap();
    let mut shell = ClientShellConfig::from_config(&local)
        .with_keybinding_source(ClientShellKeybindingSource::Endpoint);
    shell
        .apply_snapshot_keybindings(Some(&profile), &[])
        .unwrap();
    let labels = |shell: &ClientShellConfig| {
        let keybinds = &shell.keybinds.keybinds;
        (
            shell.keybinds.prefix.clone(),
            keybinds.new_tab.label(),
            keybinds.history_back.label(),
            keybinds.mark_unread.label(),
        )
    };
    let profile_prefix = vec![(KeyCode::Char('x'), KeyModifiers::CONTROL)];
    assert_eq!(
        labels(&shell),
        (
            profile_prefix.clone(),
            Some("prefix+n".to_owned()),
            Some("alt+left".to_owned()),
            None
        )
    );

    // The local config changes while the profile is in effect.
    let next: Config = toml::from_str(
        r#"
[keys]
prefix = "ctrl+b"
new_tab = "prefix+t"
history_back = "alt+up"
mark_unread = "prefix+u"
"#,
    )
    .unwrap();
    let reloaded = (
        profile_prefix,
        Some("prefix+n".to_owned()),
        Some("alt+up".to_owned()),
        Some("prefix+u".to_owned()),
    );
    assert!(shell.apply_live_config(&next, &[], &[]).is_empty());
    assert_eq!(labels(&shell), reloaded);
    // The profile is applied again after a reload and with every change
    // of it: the reloaded actions stay.
    shell
        .apply_snapshot_keybindings(Some(&profile), &[])
        .unwrap();
    assert_eq!(labels(&shell), reloaded);

    // A `[keys]` section that failed to load changes nothing.
    shell.apply_live_config(&Config::default(), &[], &["keys".to_owned()]);
    assert_eq!(labels(&shell), reloaded);
}
