use super::*;

impl ClientShellState {
    pub(super) fn active_endpoint_workspace_at(&self, point: (u16, u16)) -> Option<String> {
        self.hits
            .workspaces
            .iter()
            .find(|hit| {
                hit.endpoint_id == self.active_endpoint_id && super::contains(hit.rect, point)
            })
            .map(|hit| hit.workspace_id.clone())
    }

    pub(super) fn endpoint_workspace_is_draggable(&self, press: &ClientWorkspacePress) -> bool {
        !press.mirror
            && press.endpoint_id == self.active_endpoint_id
            && self
                .snapshot
                .as_deref()
                .and_then(|snapshot| {
                    snapshot
                        .workspaces
                        .iter()
                        .find(|workspace| workspace.workspace_id == press.workspace_id)
                })
                .is_some_and(|workspace| {
                    !workspace
                        .worktree
                        .as_ref()
                        .is_some_and(|worktree| worktree.is_linked_worktree)
                })
    }

    pub(super) fn finish_endpoint_workspace_press(
        &mut self,
        press: ClientWorkspacePress,
        outcome: &mut ClientShellInput,
    ) {
        self.focus_or_activate(
            press.endpoint_id,
            ClientEndpointFocusTarget::Workspace(press.workspace_id),
            outcome,
        );
    }

    pub(super) fn handle_endpoint_machine_click(
        &mut self,
        point: (u16, u16),
        outcome: &mut ClientShellInput,
    ) -> bool {
        let Some(hit) = self
            .hits
            .machines
            .iter()
            .find(|hit| super::contains(hit.rect, point))
        else {
            return false;
        };
        let endpoint_id = hit.endpoint_id.clone();
        let collapse_toggle = super::contains(hit.collapse_toggle, point);
        if collapse_toggle || endpoint_id == self.active_endpoint_id {
            if !self.collapsed_endpoints.remove(&endpoint_id) {
                self.collapsed_endpoints.insert(endpoint_id.clone());
            }
            outcome.repaint = true;
            if !collapse_toggle && endpoint_id.is_local() {
                self.activate_endpoint(endpoint_id, outcome);
            }
        } else if endpoint_id.is_local() || self.endpoint_is_online(&endpoint_id) {
            self.ask_for_endpoint_activation(endpoint_id, None, outcome);
        } else {
            let label = self.endpoint_label(&endpoint_id).to_owned();
            self.receive_endpoint_unavailable(format!("{label} is not ready"));
            outcome.repaint = true;
        }
        true
    }

    pub(super) fn handle_endpoint_agent_click(
        &mut self,
        point: (u16, u16),
        outcome: &mut ClientShellInput,
    ) -> bool {
        let Some((endpoint_id, pane_id)) = self
            .hits
            .endpoint_agents
            .iter()
            .find(|(rect, _, _)| super::contains(*rect, point))
            .map(|(_, endpoint_id, pane_id)| (endpoint_id.clone(), pane_id.clone()))
        else {
            return false;
        };
        self.focus_or_activate(
            endpoint_id,
            ClientEndpointFocusTarget::Pane(pane_id),
            outcome,
        );
        true
    }

    pub(super) fn handle_endpoint_navigation(
        &mut self,
        action: crate::input::KeybindAction,
        outcome: &mut ClientShellInput,
    ) -> bool {
        use crate::input::KeybindAction;
        if matches!(
            action,
            KeybindAction::PreviousWorkspace | KeybindAction::NextWorkspace
        ) && self.multi_endpoint_active()
        {
            let workspaces = self
                .endpoints
                .iter()
                .filter(|endpoint| endpoint.status == ClientEndpointStatus::Online)
                .flat_map(|endpoint| {
                    endpoint
                        .snapshot
                        .as_deref()
                        .map_or_else(Vec::new, |snapshot| {
                            space_groups::workspace_entries(snapshot, &HashSet::new())
                                .into_iter()
                                .filter_map(|entry| {
                                    snapshot.workspaces.get(entry.index).map(|workspace| {
                                        (
                                            endpoint.endpoint_id.clone(),
                                            workspace.workspace_id.clone(),
                                        )
                                    })
                                })
                                .collect()
                        })
                })
                .collect::<Vec<_>>();
            if workspaces.is_empty() {
                return true;
            }
            let focused = self
                .snapshot
                .as_deref()
                .and_then(|snapshot| snapshot.focused_workspace_id.as_deref());
            let current = workspaces.iter().position(|(endpoint_id, workspace_id)| {
                endpoint_id == &self.active_endpoint_id && Some(workspace_id.as_str()) == focused
            });
            let next = match (current, action) {
                (Some(index), KeybindAction::PreviousWorkspace) => {
                    (index + workspaces.len() - 1) % workspaces.len()
                }
                (Some(index), KeybindAction::NextWorkspace) => (index + 1) % workspaces.len(),
                (None, KeybindAction::PreviousWorkspace) => workspaces.len() - 1,
                (None, KeybindAction::NextWorkspace) => 0,
                _ => unreachable!("endpoint workspace navigation"),
            };
            let (endpoint_id, workspace_id) = workspaces[next].clone();
            self.focus_or_activate(
                endpoint_id,
                ClientEndpointFocusTarget::Workspace(workspace_id),
                outcome,
            );
            return true;
        }
        if matches!(
            action,
            KeybindAction::PreviousAgent | KeybindAction::NextAgent | KeybindAction::FocusAgent(_)
        ) {
            let agents = super::aggregate_navigation::online_agent_rows(
                &self.endpoints,
                &self.active_endpoint_id,
                self.config.agent_panel_sort,
            );
            if agents.is_empty() {
                return true;
            }
            let next = match action {
                KeybindAction::FocusAgent(index) => {
                    if index >= agents.len() {
                        return true;
                    }
                    index
                }
                KeybindAction::PreviousAgent | KeybindAction::NextAgent => {
                    let focused = self
                        .endpoints
                        .iter()
                        .find(|endpoint| endpoint.endpoint_id == self.active_endpoint_id)
                        .and_then(|endpoint| endpoint.snapshot.as_deref())
                        .and_then(|snapshot| snapshot.focused_pane_id.as_deref());
                    let current = agents.iter().position(|row| {
                        row.endpoint.endpoint_id == &self.active_endpoint_id
                            && Some(row.agent.pane_id.as_str()) == focused
                    });
                    match (current, action) {
                        (Some(index), KeybindAction::PreviousAgent) => {
                            (index + agents.len() - 1) % agents.len()
                        }
                        (Some(index), KeybindAction::NextAgent) => (index + 1) % agents.len(),
                        (None, KeybindAction::PreviousAgent) => agents.len() - 1,
                        _ => 0,
                    }
                }
                _ => unreachable!("endpoint agent navigation"),
            };
            let row = &agents[next];
            let endpoint_id = row.endpoint.endpoint_id.clone();
            let pane_id = row.agent.pane_id.clone();
            let scroll = self.agent_scroll;
            let highlighted = self.pending_workspace_highlight.is_some();
            if self.focus_or_activate(
                endpoint_id.clone(),
                ClientEndpointFocusTarget::Pane(pane_id.clone()),
                outcome,
            ) {
                if endpoint_id == self.active_endpoint_id {
                    self.reveal_endpoint_agent(&endpoint_id, &pane_id, self.hits.agent_body.height);
                } else {
                    self.pending_agent_reveal = Some((endpoint_id, pane_id));
                }
                // A focus change on one machine repaints only when the sidebar changes.
                outcome.repaint |=
                    highlighted || self.agent_scroll != scroll || self.multi_endpoint_active();
            }
            return true;
        }
        false
    }

    pub(super) fn activate_endpoint(
        &mut self,
        endpoint_id: ClientEndpointId,
        outcome: &mut ClientShellInput,
    ) -> bool {
        self.pending_workspace_highlight = None;
        self.pending_agent_reveal = None;
        let online = self.endpoint_is_online(&endpoint_id);
        if !online && !endpoint_id.is_local() {
            let label = self.endpoint_label(&endpoint_id).to_owned();
            self.receive_endpoint_unavailable(format!("{label} is not ready"));
            outcome.repaint = true;
            return false;
        }
        if (endpoint_id.is_local() && (self.multi_endpoint_active() || !online))
            || endpoint_id != self.active_endpoint_id
        {
            self.ask_for_endpoint_activation(endpoint_id, None, outcome);
        }
        true
    }

    /// Asks the client loop to activate `endpoint_id`, with `target` focused
    /// there. From here on the loop holds an activation, until it says it
    /// holds none: a navigation the shell follows that comes next, even in
    /// the same batch of input, goes through the loop as well.
    pub(super) fn ask_for_endpoint_activation(
        &mut self,
        endpoint_id: ClientEndpointId,
        target: Option<ClientEndpointFocusTarget>,
        outcome: &mut ClientShellInput,
    ) {
        self.endpoint_activation_held = true;
        outcome.actions.push(ClientShellAction::ActivateEndpoint {
            endpoint_id,
            target,
        });
    }

    /// Step through the pane visit history, skipping offline machines.
    pub(super) fn history_step(&mut self, forward: bool, outcome: &mut ClientShellInput) {
        let online: HashSet<ClientEndpointId> = self
            .endpoints
            .iter()
            .filter(|endpoint| endpoint.status == ClientEndpointStatus::Online)
            .map(|endpoint| endpoint.endpoint_id.clone())
            .collect();
        let step = if forward {
            self.history.forward(&online)
        } else {
            self.history.back(&online)
        };
        let Some(step) = step else {
            let body = if forward {
                "No later pane in history"
            } else {
                "No earlier pane in history"
            };
            outcome.repaint |= self.push_endpoint_notice(
                ClientEndpointNoticeKind::Unavailable,
                "history",
                "History",
                body,
            );
            return;
        };
        let dispatched = outcome.actions.len();
        let endpoint_id = step.visit.endpoint_id.clone();
        let target = ClientEndpointFocusTarget::Pane(step.visit.pane_id.clone());
        // An earlier step still tracked as an activation is replaced by this
        // one, in the history and in the client loop alike.
        self.history_handoff = None;
        let navigating = self.focus_or_activate_with_kind(
            endpoint_id,
            target,
            PendingEndpointKind::HistoryStep { step: step.clone() },
            outcome,
        );
        match outcome.actions.get(dispatched) {
            // The step's own focus request: its result ends the wait.
            Some(ClientShellAction::Endpoint { .. }) if navigating => {}
            Some(ClientShellAction::ActivateEndpoint { .. }) if navigating => {
                self.history_handoff = Some(step);
            }
            // No request and no activation: the step was refused on the spot.
            _ => self.fail_history_step(&step, false),
        }
        outcome.repaint = true;
    }

    /// The endpoint carried out the focus request of a history step. The
    /// snapshot that shows the step's target may have arrived ahead of this
    /// answer, where it confirmed nothing, so what is presented now is
    /// checked against the target.
    pub(super) fn answer_history_step(&mut self, step: &super::history::Step) {
        let visit = &step.visit;
        let shown = self.active_endpoint_id == visit.endpoint_id
            && self.snapshot.as_deref().is_some_and(|snapshot| {
                snapshot.boot_id == visit.boot_id
                    && snapshot.focused_pane_id.as_deref() == Some(visit.pane_id.as_str())
            });
        self.history.traversal_answered(step, shown);
    }

    /// The navigation of a history step failed or was refused. The cursor
    /// returns to where the step began, and the pane now presented, which
    /// went unrecorded while the step was awaited, is observed again.
    pub(super) fn fail_history_step(&mut self, step: &super::history::Step, pane_closed: bool) {
        self.history.traversal_failed(step, pane_closed);
        if let Some(snapshot) = self.snapshot.as_deref() {
            self.history.observe(
                &self.active_endpoint_id,
                &snapshot.boot_id,
                snapshot.focused_pane_id.as_deref(),
            );
        }
    }

    /// The client loop completed an activation of `endpoint_id` that had
    /// `focus` as its target. A history step that left as an activation and
    /// leads there was carried out, as an answered focus request says of a
    /// step on the presented endpoint. A step that leads elsewhere is not
    /// this activation's: it replaced the one that began it, and the loop
    /// takes it up next. Presenting the endpoint says nothing of the kind:
    /// the loop shows a handoff's target before the handoff completes, and
    /// shows the source of a reversed one, where a pane equal to the step's
    /// target may be focused, before the step's own activation begins.
    pub(crate) fn receive_endpoint_activated(
        &mut self,
        endpoint_id: &ClientEndpointId,
        focus: Option<&ClientEndpointFocusTarget>,
    ) {
        let carried_out = self.history_handoff.take_if(|step| {
            &step.visit.endpoint_id == endpoint_id
                && matches!(focus, Some(ClientEndpointFocusTarget::Pane(pane_id))
                    if pane_id == &step.visit.pane_id)
        });
        if let Some(step) = carried_out {
            self.answer_history_step(&step);
        }
    }

    /// The client loop says, on every turn, whether it holds an endpoint
    /// activation: one under way, one about to begin, or one it put off.
    /// While it does, a history step or a jump goes through the loop, which
    /// takes no request during a handoff (see `focus_or_activate_with_kind`),
    /// and a jump is not answered `focused`, because the client may be about
    /// to leave the pane. Once it holds none, a history step that left as an
    /// activation and was not carried out will not arrive: the activation
    /// failed, was given up, or ended on another selection.
    pub(crate) fn receive_endpoint_activations_held(&mut self, held: bool) {
        self.endpoint_activation_held = held;
        if held {
            return;
        }
        if let Some(step) = self.history_handoff.take() {
            self.fail_history_step(&step, false);
        }
    }

    pub(super) fn focus_or_activate(
        &mut self,
        endpoint_id: ClientEndpointId,
        target: ClientEndpointFocusTarget,
        outcome: &mut ClientShellInput,
    ) -> bool {
        self.focus_or_activate_with_kind(endpoint_id, target, PendingEndpointKind::Generic, outcome)
    }

    /// `kind` tracks the focus request on the active endpoint; activating another
    /// endpoint carries the target through its own handoff instead. A kind
    /// other than `Generic` is a navigation the shell follows to its end: a
    /// history step or a jump.
    pub(super) fn focus_or_activate_with_kind(
        &mut self,
        endpoint_id: ClientEndpointId,
        target: ClientEndpointFocusTarget,
        kind: PendingEndpointKind,
        outcome: &mut ClientShellInput,
    ) -> bool {
        #[cfg(windows)]
        if !self.notification_target_is_current(&endpoint_id, &target) {
            return false;
        }
        self.pending_workspace_highlight = None;
        self.pending_agent_reveal = None;
        let online = self.endpoint_is_online(&endpoint_id);
        if !online && !endpoint_id.is_local() {
            let label = self.endpoint_label(&endpoint_id).to_owned();
            self.receive_endpoint_unavailable(format!("{label} is not ready"));
            outcome.repaint = true;
            return false;
        }
        // While the client loop holds an activation it takes no request for
        // the endpoint presented and would cancel this one. A navigation the
        // shell follows goes through the loop then, which puts it in that
        // activation's place or behind it, or sends it back as the request
        // when the activation is over after all.
        let through_loop =
            self.endpoint_activation_held && !matches!(kind, PendingEndpointKind::Generic);
        // Local can still be displayed while a remote activation is pending.
        // Route explicit selections through the runtime so they can cancel that handoff.
        if endpoint_id == self.active_endpoint_id
            && !(endpoint_id.is_local() && (self.multi_endpoint_active() || !online))
            && !through_loop
        {
            let method = match target {
                ClientEndpointFocusTarget::Workspace(workspace_id) => {
                    crate::api::schema::Method::WorkspaceFocus(
                        crate::api::schema::WorkspaceTarget { workspace_id },
                    )
                }
                ClientEndpointFocusTarget::Tab(tab_id) => {
                    crate::api::schema::Method::TabFocus(crate::api::schema::TabTarget { tab_id })
                }
                ClientEndpointFocusTarget::Pane(pane_id) => {
                    crate::api::schema::Method::PaneFocus(crate::api::schema::PaneTarget {
                        pane_id,
                    })
                }
                #[cfg(windows)]
                ClientEndpointFocusTarget::Notification { pane_id, .. } => {
                    crate::api::schema::Method::PaneFocus(crate::api::schema::PaneTarget {
                        pane_id,
                    })
                }
            };
            self.push_endpoint_method_with_kind(method, kind, outcome);
        } else {
            self.ask_for_endpoint_activation(endpoint_id, Some(target), outcome);
        }
        true
    }
}
