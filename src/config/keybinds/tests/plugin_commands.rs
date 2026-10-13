//! `type = "plugin"` key commands.

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
    assert_eq!(binding.action, CustomCommandAction::PluginAction);
    assert_eq!(binding.command, "gjermundgaraba.clankersnip.palette");
    assert_eq!(binding.width, Some(PopupSize::Cells(90)));
    assert_eq!(binding.height, Some(PopupSize::Percent(80)));
}
