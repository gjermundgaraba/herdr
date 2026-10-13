//! `type = "plugin"` key commands, and command types this build does not know.

use super::*;

#[test]
fn custom_plugin_command_preserves_popup_size_overrides() {
    let config: Config = toml::from_str(
        r#"
[[keys.command]]
key = "prefix+s"
command = "gjermundgaraba.clankersnip.palette"
type = "plugin"
width = 90
height = "80%"
"#,
    )
    .unwrap();
    assert!(config.collect_diagnostics().is_empty());
    let bindings = config.keybinds();
    let binding = &bindings.custom_commands[0];
    assert_eq!(binding.action, CustomCommandAction::Plugin);
    assert_eq!(binding.command, "gjermundgaraba.clankersnip.palette");
    assert_eq!(binding.width, Some(PopupSize::Cells(90)));
    assert_eq!(binding.height, Some(PopupSize::Percent(80)));
}

#[test]
fn unknown_custom_command_type_disables_only_that_binding() {
    let config: Config = toml::from_str(
        r#"
[update]
version_check = false

[[keys.command]]
key = "prefix+s"
command = "example.plugin.palette"
type = "plugin_pane"

[[keys.command]]
key = "prefix+s"
command = "echo valid"
"#,
    )
    .unwrap();
    assert!(!config.update.version_check);
    let diagnostics = config.collect_diagnostics();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert!(diagnostics[0].contains("unknown custom command type: keys.command[0].type"));
    let bindings = config.keybinds();
    assert_eq!(bindings.custom_commands.len(), 1);
    assert_eq!(bindings.custom_commands[0].command, "echo valid");
}
