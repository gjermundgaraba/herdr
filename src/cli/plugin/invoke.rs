//! `herdr plugin invoke`, which runs any manifest entrypoint through
//! `plugin.invoke` with an optional target space, tab, or pane.

use super::{print_plugin_response, required_value};
use crate::api::schema::{Method, PluginInvokeParams};

pub(super) fn plugin_invoke(args: &[String]) -> std::io::Result<i32> {
    const USAGE: &str =
        "usage: herdr plugin invoke <plugin_id.entry_id> [--workspace ID] [--tab ID] [--pane ID]";
    // Plugin ids may contain dots and entry ids cannot.
    let Some((plugin_id, entry_id)) = args.first().and_then(|target| target.rsplit_once('.'))
    else {
        eprintln!("{USAGE}");
        return Ok(2);
    };
    let mut params = PluginInvokeParams {
        plugin_id: plugin_id.to_owned(),
        entry_id: entry_id.to_owned(),
        workspace_id: None,
        tab_id: None,
        pane_id: None,
        selection: None,
    };
    let mut index = 1;
    while index < args.len() {
        let flag = args[index].as_str();
        let slot = match flag {
            "--workspace" => &mut params.workspace_id,
            "--tab" => &mut params.tab_id,
            "--pane" => &mut params.pane_id,
            other => {
                eprintln!("unknown option: {other}");
                eprintln!("{USAGE}");
                return Ok(2);
            }
        };
        let Some(value) = required_value(args, &mut index, flag) else {
            return Ok(2);
        };
        *slot = Some(value);
    }
    print_plugin_response(Method::PluginInvoke(params))
}
