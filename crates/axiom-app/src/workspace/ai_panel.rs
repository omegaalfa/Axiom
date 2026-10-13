use super::*;

impl WorkspaceView {
    pub(super) fn render_ai_chat_conversation(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme();
        let assistant_label = if self.model_label.trim().is_empty() || self.model_label == "Model" {
            "Axiom".to_owned()
        } else {
            self.model_label.clone()
        };
        div()
            .id("ai-chat-conversation")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.chat_scroll_handle)
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .text_color(t.text_secondary)
            .when(self.chat_messages.is_empty(), |this| {
                this.items_center()
                    .justify_center()
                    .child(div().text_size(px(28.)).text_color(t.accent).child("✦"))
                    .child(div().text_color(t.text_primary).child("Ask Axiom anything"))
                    .child("Understand code, diagnose problems,")
                    .child("or start an agent task.")
            })
            .children(self.chat_messages.iter().map(|message| {
                let message_id = message.id;
                let message_content = message.content.clone();
                div()
                    .id(SharedString::from(format!(
                        "ai-chat-message-{}",
                        message.id
                    )))
                    .w_full()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .when(message.role == ChatRole::User, |this| {
                        this.bg(t.editor_background)
                            .rounded(metrics().border_radius_small)
                            .px_2()
                            .py_1()
                    })
                    .child(
                        div()
                            .w_full()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .text_color(if message.role == ChatRole::Assistant {
                                        t.accent
                                    } else {
                                        t.text_primary
                                    })
                                    .child(if message.role == ChatRole::Assistant {
                                        axiom_icon(t.accent)
                                    } else {
                                        user_icon(t.text_muted)
                                    })
                                    .child(if message.role == ChatRole::Assistant {
                                        assistant_label.clone()
                                    } else {
                                        "You".to_owned()
                                    }),
                            ),
                    )
                    .when_some(
                        (message.role == ChatRole::Assistant
                            && message
                                .thinking
                                .as_ref()
                                .is_some_and(|thinking| !thinking.trim().is_empty()))
                        .then(|| message.thinking.clone().unwrap()),
                        |this, thinking| {
                            let expanded = self.thinking_expanded.contains(&message.id);
                            let message_id = message.id;
                            this.child(
                                div()
                                    .id(SharedString::from(format!("ai-thinking-{}", message_id)))
                                    .px_2()
                                    .py_1()
                                    .rounded(metrics().border_radius_small)
                                    .bg(t.elevated_surface)
                                    .text_color(t.text_muted)
                                    .cursor(CursorStyle::PointingHand)
                                    .hover(move |s| s.bg(t.hover))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if !this.thinking_expanded.insert(message_id) {
                                            this.thinking_expanded.remove(&message_id);
                                        }
                                        cx.notify();
                                    }))
                                    .child(if expanded {
                                        div()
                                            .child("Thinking ▾")
                                            .child(render_thinking_text(&thinking, t.text_muted))
                                    } else {
                                        div().child("Thinking ▸")
                                    }),
                            )
                        },
                    )
                    .child(match message.role {
                        ChatRole::Assistant => render_assistant_markdown(&message.content, cx),
                        _ => div().w_full().min_w_0().child(message.content.clone()),
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .mt_1()
                            .px_1()
                            .when(self.chat_copy_notice == Some(message_id), |this| {
                                this.child(
                                    div()
                                        .px_1()
                                        .text_size(px(11.))
                                        .text_color(t.accent)
                                        .child("Copied"),
                                )
                            })
                            .child(
                                div()
                                    .id(SharedString::from(format!(
                                        "ai-copy-message-{}",
                                        message_id
                                    )))
                                    .px_1()
                                    .cursor(CursorStyle::PointingHand)
                                    .hover(move |s| s.bg(t.hover))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            message_content.clone(),
                                        ));
                                        this.show_chat_copy_notice(message_id, cx);
                                        this.status = "Copied".into();
                                    }))
                                    .tooltip(|_, cx| tooltip("Copy message", cx))
                                    .child("⧉"),
                            ),
                    )
            }))
            .when_some(
                match &self.chat_request_state {
                    ChatRequestState::Error(message) => Some(message.clone()),
                    _ => None,
                },
                |this, message| this.child(div().text_color(t.error).child(message)),
            )
    }

    pub(super) fn show_agent_copy_notice(&mut self, message_id: u64, cx: &mut Context<Self>) {
        self.agent_copy_notice = Some(message_id);
        let entity = cx.entity();
        cx.notify();
        cx.spawn(async move |_, cx| {
            Timer::after(std::time::Duration::from_millis(1400)).await;
            let _ = entity.update(cx, |this, cx| {
                if this.agent_copy_notice == Some(message_id) {
                    this.agent_copy_notice = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(super) fn render_agent_conversation(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme();
        let assistant_label = if self.model_label.trim().is_empty() || self.model_label == "Model" {
            "Axiom".to_owned()
        } else {
            self.model_label.clone()
        };
        div()
            .id("ai-agent-conversation")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.agent_scroll_handle)
            .p_3()
            .text_color(t.text_secondary)
            .when_some(
                agent_history_status_text(
                    self.active_agent_conversation_id.is_some(),
                    &self.agent_history_save_state,
                ),
                |this, status| {
                    this.child(
                        div()
                            .id("ai-agent-history-save-status")
                            .flex()
                            .items_center()
                            .justify_between()
                            .text_xs()
                            .text_color(t.text_muted)
                            .child(status)
                            .when(
                                matches!(
                                    self.agent_history_save_state,
                                    AgentHistorySaveState::Error(_)
                                        | AgentHistorySaveState::Unavailable(_)
                                ),
                                |this| {
                                    this.child(
                                        div()
                                            .id("ai-agent-history-retry")
                                            .px_1()
                                            .cursor(CursorStyle::PointingHand)
                                            .text_color(t.accent)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.retry_agent_history_save(cx)
                                            }))
                                            .child("Retry"),
                                    )
                                },
                            ),
                    )
                },
            )
            .when_some(
                self.active_agent_conversation_id
                    .as_ref()
                    .and_then(|_| tool_context_message(&self.agent_tool_summaries)),
                |this, context| {
                    this.child(
                        div()
                            .id("ai-agent-history-tool-context")
                            .px_2()
                            .py_1()
                            .rounded(metrics().border_radius_small)
                            .bg(t.elevated_surface)
                            .text_xs()
                            .text_color(t.text_muted)
                            .child(context),
                    )
                },
            )
            .when(self.agent_messages.is_empty(), |this| {
                this.items_center()
                    .justify_center()
                    .child("Start an agent task")
            })
            .children(self.agent_messages.iter().map(|message| {
                let message_id = message.id;
                let message_content = message.content.clone();
                div()
                    .id(SharedString::from(format!("ai-agent-message-{message_id}")))
                    .w_full()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .when(message.role == ChatRole::User, |this| {
                        this.bg(t.editor_background)
                            .rounded(metrics().border_radius_small)
                            .px_2()
                            .py_1()
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .text_color(if message.role == ChatRole::Assistant {
                                t.accent
                            } else {
                                t.text_primary
                            })
                            .child(if message.role == ChatRole::Assistant {
                                axiom_icon(t.accent)
                            } else {
                                user_icon(t.text_muted)
                            })
                            .child(if message.role == ChatRole::Assistant {
                                assistant_label.clone()
                            } else {
                                "You".to_owned()
                            }),
                    )
                    .when_some(
                        (message.role == ChatRole::Assistant
                            && message
                                .thinking
                                .as_ref()
                                .is_some_and(|thinking| !thinking.trim().is_empty()))
                        .then(|| message.thinking.clone().unwrap()),
                        |this, thinking| {
                            this.child(
                                div()
                                    .px_2()
                                    .py_1()
                                    .bg(t.elevated_surface)
                                    .text_color(t.text_muted)
                                    .child(render_thinking_text(&thinking, t.text_muted)),
                            )
                        },
                    )
                    .child(match message.role {
                        ChatRole::Assistant => render_assistant_markdown(&message.content, cx),
                        _ => div().w_full().min_w_0().child(message.content.clone()),
                    })
                    .when(message.role == ChatRole::Assistant, |this| {
                        this.child(
                            div()
                                .flex()
                                .items_center()
                                .mt_1()
                                .px_1()
                                .when(self.agent_copy_notice == Some(message_id), |this| {
                                    this.child(
                                        div()
                                            .px_1()
                                            .text_size(px(11.))
                                            .text_color(t.accent)
                                            .child("Copied"),
                                    )
                                })
                                .child(
                                    div()
                                        .id(SharedString::from(format!(
                                            "ai-agent-copy-message-{message_id}"
                                        )))
                                        .px_1()
                                        .cursor(CursorStyle::PointingHand)
                                        .hover(move |s| s.bg(t.hover))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            cx.stop_propagation();
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                message_content.clone(),
                                            ));
                                            this.show_agent_copy_notice(message_id, cx);
                                            this.status = "Copied".into();
                                        }))
                                        .tooltip(|_, cx| tooltip("Copy message", cx))
                                        .child("⧉"),
                                ),
                        )
                    })
            }))
            .when(
                agent_approval_visible(self.ai_panel_mode, &self.agent_pending_approval),
                |this| {
                    let approval = self
                        .agent_pending_approval
                        .clone()
                        .expect("approval visibility checked");
                    let approve_approval_id = approval.approval_id;
                    let allow_for_request_approval_id = approval.approval_id;
                    let deny_approval_id = approval.approval_id;
                    this.child(
                        div()
                            .id("ai-agent-approval-card")
                            .w_full()
                            .mt_2()
                            .p_2()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .rounded(metrics().border_radius_small)
                            .bg(t.elevated_surface)
                            .border_1()
                            .border_color(t.border)
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .text_color(t.text_primary)
                                    .child("Approval required")
                                    .child(
                                        div()
                                            .ml_auto()
                                            .text_size(px(11.))
                                            .text_color(t.text_muted)
                                            .child("Tool"),
                                    ),
                            )
                            .child(
                                div()
                                    .id("ai-agent-approval-tool")
                                    .text_color(t.accent)
                                    .child(approval.tool_name.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(t.text_muted)
                                    .child("Reason"),
                            )
                            .child(
                                div()
                                    .id("ai-agent-approval-reason")
                                    .text_color(t.text_secondary)
                                    .child(approval.reason.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(t.text_muted)
                                    .child("Arguments"),
                            )
                            .child(
                                div()
                                    .id("ai-agent-approval-arguments")
                                    .max_h(px(120.))
                                    .overflow_hidden()
                                    .p_2()
                                    .rounded(metrics().border_radius_small)
                                    .bg(t.editor_background)
                                    .text_size(px(11.))
                                    .text_color(t.text_secondary)
                                    .child(approval.arguments_preview.clone()),
                            )
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .justify_end()
                                    .child(
                                        div()
                                            .id("ai-agent-approval-deny")
                                            .px_3()
                                            .py_1()
                                            .rounded(metrics().border_radius_small)
                                            .border_1()
                                            .border_color(t.border)
                                            .cursor(CursorStyle::PointingHand)
                                            .hover(move |s| s.bg(t.hover))
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                cx.stop_propagation();
                                                this.submit_agent_approval(
                                                    deny_approval_id,
                                                    AgentApprovalDecision::Deny,
                                                    cx,
                                                );
                                            }))
                                            .child("Deny"),
                                    )
                                    .child(
                                        div()
                                            .id("ai-agent-approval-approve")
                                            .px_3()
                                            .py_1()
                                            .rounded(metrics().border_radius_small)
                                            .bg(t.accent)
                                            .text_color(t.text_primary)
                                            .cursor(CursorStyle::PointingHand)
                                            .hover(move |s| s.bg(t.accent_hover))
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                cx.stop_propagation();
                                                this.submit_agent_approval(
                                                    approve_approval_id,
                                                    AgentApprovalDecision::Approve,
                                                    cx,
                                                );
                                            }))
                                            .child("Approve once"),
                                    )
                                    .child(
                                        div()
                                            .id("ai-agent-approval-allow-for-request")
                                            .px_3()
                                            .py_1()
                                            .rounded(metrics().border_radius_small)
                                            .border_1()
                                            .border_color(t.border)
                                            .cursor(CursorStyle::PointingHand)
                                            .hover(move |s| s.bg(t.hover))
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                cx.stop_propagation();
                                                this.submit_agent_approval(
                                                    allow_for_request_approval_id,
                                                    AgentApprovalDecision::AllowForRequest,
                                                    cx,
                                                );
                                            }))
                                            .child("Allow for this request"),
                                    ),
                            ),
                    )
                },
            )
            .when(
                agent_activity_indicator_visible(self.agent_ui_state, &self.agent_pending_approval),
                |this| {
                    let dots = agent_activity_dots(self.agent_activity_animation_phase);
                    this.child(
                        div()
                            .id("ai-agent-generating-indicator")
                            .flex()
                            .items_center()
                            .gap_1()
                            .text_color(t.text_muted)
                            .child(div().w(px(72.)).child(self.agent_activity_phase.label()))
                            .child(div().w(px(36.)).child(dots)),
                    )
                },
            )
            .when_some(self.agent_status.clone(), |this, status| {
                this.child(div().text_color(t.text_muted).child(status))
            })
    }

    pub(super) fn is_ai_generation_active(&self) -> bool {
        match self.ai_panel_mode {
            AiPanelMode::Chat => matches!(self.chat_request_state, ChatRequestState::Sending),
            AiPanelMode::Agent => matches!(self.agent_ui_state, AgentUiState::Running),
            AiPanelMode::Memory => false,
        }
    }

    pub fn set_memory_service(
        &mut self,
        service: Option<axiom_agent::SharedMemoryService>,
        cx: &mut Context<Self>,
    ) {
        self.memory_service = service;
        self.memory_ui_generation = self.memory_ui_generation.wrapping_add(1);
        self.memory_ui_snapshot = None;
        if self.ai_panel_mode == AiPanelMode::Memory {
            self.refresh_memory(cx);
        } else {
            self.memory_ui_state = MemoryUiState::Unavailable;
            cx.notify();
        }
    }

    pub(super) fn refresh_memory(&mut self, cx: &mut Context<Self>) {
        self.memory_ui_generation = self.memory_ui_generation.wrapping_add(1);
        let generation = self.memory_ui_generation;
        let scope = self.memory_ui_scope;
        let service = self.memory_service.clone();
        self.memory_ui_state = MemoryUiState::Loading;
        self.memory_ui_snapshot = None;
        cx.spawn(async move |this, cx| {
            let result = background_executor()
                .spawn(async move {
                    let service = service.ok_or(())?;
                    if !service.is_available() {
                        return Err(());
                    }
                    let snapshot = service
                        .snapshot(scope, MAX_MEMORY_UI_RESULTS)
                        .map_err(|_| ())?;
                    let recent = snapshot.recent;
                    let history = snapshot.history;
                    let decisions = history
                        .iter()
                        .filter(|entry| {
                            entry.category.as_ref()
                                == Some(&axiom_agent::MemoryCategory::Decision)
                        })
                        .take(MAX_MEMORY_UI_RESULTS)
                        .cloned()
                        .collect();
                    let gotchas = history
                        .iter()
                        .filter(|entry| {
                            entry.category.as_ref()
                                == Some(&axiom_agent::MemoryCategory::Gotcha)
                        })
                        .take(MAX_MEMORY_UI_RESULTS)
                        .cloned()
                        .collect();
                    Ok(MemoryUiSnapshot {
                        scope,
                        sessions: history,
                        recent,
                        decisions,
                        gotchas,
                    })
                })
                .await;
            let _ = this.update(cx, |workspace, cx| {
                if !memory_ui_request_is_current(
                    workspace.memory_ui_generation,
                    workspace.memory_ui_scope,
                    workspace.ai_panel_mode,
                    generation,
                    scope,
                ) {
                    return;
                }
                match result {
                    Ok(snapshot) => {
                        workspace.memory_ui_state = memory_ui_state_for_snapshot(&snapshot);
                        workspace.memory_ui_snapshot = Some(snapshot);
                    }
                    Err(()) => {
                        workspace.memory_ui_state = MemoryUiState::Unavailable;
                        workspace.memory_ui_snapshot = None;
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn set_memory_ui_tab(&mut self, tab: MemoryUiTab, cx: &mut Context<Self>) {
        self.memory_ui_tab = tab;
        cx.notify();
    }

    pub(super) fn delete_memory_entry_from_ui(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(service) = self.memory_service.clone() else {
            self.status = "Persistent memory is unavailable".into();
            cx.notify();
            return;
        };
        let scope = self.memory_ui_scope;
        self.memory_ui_generation = self.memory_ui_generation.wrapping_add(1);
        let generation = self.memory_ui_generation;
        if let Some(snapshot) = self.memory_ui_snapshot.as_mut() {
            for entries in [
                &mut snapshot.sessions,
                &mut snapshot.recent,
                &mut snapshot.decisions,
                &mut snapshot.gotchas,
            ] {
                entries.retain(|entry| entry.id.as_deref() != Some(id.as_str()));
            }
            self.memory_ui_state = memory_ui_state_for_snapshot(snapshot);
        }
        self.status = "Deleting memory entry…".into();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = background_executor()
                .spawn(async move { service.delete_memory_entry(scope, &id) })
                .await;
            let _ = this.update(cx, |workspace, cx| {
                if workspace.memory_ui_generation != generation {
                    return;
                }
                workspace.status = match result {
                    Ok(true) => "Memory entry deleted".into(),
                    Ok(false) => "Memory entry was already absent".into(),
                    Err(_) => "Could not delete memory entry; storage outcome may be unknown".into(),
                };
                workspace.refresh_memory(cx);
            });
        })
        .detach();
    }

    pub(super) fn request_clear_memory(&mut self, cx: &mut Context<Self>) {
        self.memory_clear_confirmation = true;
        cx.notify();
    }

    pub(super) fn cancel_clear_memory(&mut self, cx: &mut Context<Self>) {
        self.memory_clear_confirmation = false;
        cx.notify();
    }

    pub(super) fn clear_memory_from_ui(&mut self, cx: &mut Context<Self>) {
        self.memory_clear_confirmation = false;
        if self.memory_ui_scope != axiom_agent::MemoryScope::Workspace {
            self.status = "Clear Memory is available only for workspace scope".into();
            cx.notify();
            return;
        }
        let Some(service) = self.memory_service.clone() else {
            self.status = "Persistent memory is unavailable".into();
            cx.notify();
            return;
        };
        let scope = self.memory_ui_scope;
        self.memory_ui_generation = self.memory_ui_generation.wrapping_add(1);
        let generation = self.memory_ui_generation;
        if let Some(snapshot) = self.memory_ui_snapshot.as_mut() {
            snapshot.sessions.clear();
            snapshot.recent.clear();
            snapshot.decisions.clear();
            snapshot.gotchas.clear();
        }
        self.memory_ui_state = MemoryUiState::Empty;
        self.status = "Clearing workspace memory…".into();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = background_executor()
                .spawn(async move { service.clear_memory(scope) })
                .await;
            let _ = this.update(cx, |workspace, cx| {
                if workspace.memory_ui_generation != generation {
                    return;
                }
                workspace.status = match result {
                    Ok(removed) => format!("Cleared {removed} workspace memory entries").into(),
                    Err(_) => "Could not clear workspace memory; storage outcome may be unknown".into(),
                };
                workspace.refresh_memory(cx);
            });
        })
        .detach();
    }

    pub(super) fn render_memory_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme();
        let m = metrics();
        let workspace = cx.entity();
        let scope = self
            .memory_ui_snapshot
            .as_ref()
            .map(|snapshot| snapshot.scope)
            .unwrap_or(self.memory_ui_scope);
        let scope_label = memory_scope_label(scope);
        let entries = self
            .memory_ui_snapshot
            .as_ref()
            .map(|snapshot| memory_ui_entries_for_tab(snapshot, self.memory_ui_tab))
            .unwrap_or_default();
        let state = self.memory_ui_state;
        div()
            .id("ai-memory-panel")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_3()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child("Memory")
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_color(t.text_muted)
                                    .child(format!("Scope: {scope_label}")),
                            )
                            .when(scope == axiom_agent::MemoryScope::Workspace, |this| {
                                this.child(
                                    div()
                                        .id("ai-memory-clear")
                                        .px_2()
                                        .py_1()
                                        .rounded(m.border_radius_small)
                                        .cursor(CursorStyle::PointingHand)
                                        .text_color(t.error)
                                        .hover(move |style| style.bg(t.hover))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.request_clear_memory(cx);
                                        }))
                                        .child("Clear Memory"),
                                )
                            }),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_1()
                    .children([
                        (MemoryUiTab::Overview, "Overview"),
                        (MemoryUiTab::Sessions, "Sessions"),
                        (MemoryUiTab::Decisions, "Decisions"),
                        (MemoryUiTab::Gotchas, "Gotchas"),
                    ]
                    .into_iter()
                    .enumerate()
                    .map(|(index, (tab, label))| {
                        div()
                            .id(("ai-memory-tab", index))
                            .px_2()
                            .py_1()
                            .rounded(m.border_radius_small)
                            .cursor(CursorStyle::PointingHand)
                            .when(self.memory_ui_tab == tab, |this| {
                                this.bg(t.inactive_selection).text_color(t.text_primary)
                            })
                            .when(self.memory_ui_tab != tab, |this| {
                                this.text_color(t.text_muted)
                            })
                            .hover(move |style| style.bg(t.hover))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.set_memory_ui_tab(tab, cx);
                            }))
                            .child(label)
                    })),
            )
            .child(match state {
                MemoryUiState::Loading => div().text_color(t.text_muted).child("Loading memory…"),
                MemoryUiState::Unavailable => div()
                    .text_color(t.text_muted)
                    .child("Memory unavailable"),
                MemoryUiState::Empty => div()
                    .text_color(t.text_muted)
                    .child("No memories in this scope"),
                MemoryUiState::Ready => div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .children(entries.into_iter().take(MAX_MEMORY_UI_RESULTS).map(|entry| {
                        let content = truncate_memory_ui_content(&entry.content);
                        let id = entry.id.clone();
                        let group = SharedString::from(format!(
                            "ai-memory-entry-group-{}",
                            id.as_deref().unwrap_or("anonymous")
                        ));
                        let delete_workspace = workspace.clone();
                        div()
                            .id(SharedString::from(format!(
                                "ai-memory-entry-{}",
                                entry.id.as_deref().unwrap_or("anonymous")
                            )))
                            .group(group.clone())
                            .w_full()
                            .min_w_0()
                            .flex()
                            .items_start()
                            .gap_2()
                            .px_2()
                            .py_1()
                            .rounded(m.border_radius_small)
                            .bg(t.editor_background)
                            .text_color(t.text_primary)
                            .child(div().flex_1().min_w_0().child(content))
                            .when_some(id, |this, id| {
                                this.child(
                                    div()
                                        .id(SharedString::from(format!("ai-memory-delete-{id}")))
                                        .flex_shrink_0()
                                        .px_1()
                                        .cursor(CursorStyle::PointingHand)
                                        .text_color(t.error)
                                        .opacity(0.)
                                        .group_hover(group, |style| style.opacity(1.))
                                        .tooltip(|_, cx| tooltip("Delete memory entry", cx))
                                        .on_click(move |_, _, cx| {
                                            cx.stop_propagation();
                                            delete_workspace.update(cx, |this, cx| {
                                                this.delete_memory_entry_from_ui(id.clone(), cx);
                                            });
                                        })
                                        .child("🗑"),
                                )
                            })
                    })),
            })
    }

    pub(super) fn switch_ai_panel_mode(&mut self, mode: AiPanelMode, cx: &mut Context<Self>) {
        if self.ai_panel_mode == mode {
            return;
        }
        self.ai_panel_mode = mode;
        if mode == AiPanelMode::Chat {
            self.chat_scroll_handle.scroll_to_bottom();
            self.chat_auto_follow = true;
            self.chat_scroll_observed = false;
            self.chat_programmatic_scroll_pending = true;
        } else if mode == AiPanelMode::Agent {
            self.agent_scroll_handle.scroll_to_bottom();
        } else {
            self.refresh_memory(cx);
        }
        cx.notify();
    }

    pub(super) fn stop_ai_generation(&mut self, cx: &mut Context<Self>) {
        if self.ai_panel_mode == AiPanelMode::Agent {
            self.stop_agent_run(cx);
        } else {
            self.stop_ai_chat_generation(cx);
        }
    }

    pub(super) fn render_thinking_level_picker(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme();
        let m = metrics();
        let levels = self.selected_thinking_capability().levels;
        div()
            .id("ai-thinking-level-picker")
            .absolute()
            .bottom(px(88.))
            .right(px(72.))
            .w(px(130.))
            .p_1()
            .flex()
            .flex_col()
            .gap_1()
            .bg(t.popup_background)
            .border_1()
            .border_color(t.border)
            .rounded(m.border_radius_medium)
            .shadow_lg()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .children(levels.into_iter().enumerate().map(|(level_index, level)| {
                let selected = self.selected_thinking_level() == Some(level);
                div()
                    .id(("ai-thinking-level-option", level_index))
                    .px_2()
                    .py_1()
                    .cursor(CursorStyle::PointingHand)
                    .text_color(if selected { t.accent } else { t.text_primary })
                    .bg(if selected {
                        t.inactive_selection
                    } else {
                        t.popup_background
                    })
                    .hover(move |style| style.bg(t.hover))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            if let Some(provider) = this.active_provider.clone() {
                                this.thinking_level_preferences
                                    .insert((provider, this.model_label.clone()), level);
                                this.persist_ui_settings();
                            }
                            this.thinking_level_picker_open = false;
                            cx.notify();
                        }),
                    )
                    .child(level.label())
            }))
    }

    pub(super) fn render_ai_panel(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme();
        let m = metrics();
        let workspace = cx.entity();
        let active_context = self.chat_context_sources.first();
        let context_label = active_context.map(context_source_ui_label);
        let context_full_label = active_context.map(|source| source.label.clone());
        let context_is_active = active_context.is_some();
        div()
            .id("ai-panel")
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.thinking_level_picker_open {
                        this.thinking_level_picker_open = false;
                        cx.notify();
                    }
                }),
            )
            .relative()
            .w(ai_panel_width_for_viewport(
                self.ai_panel_width,
                window.viewport_size(),
                if self.project_panel_visible {
                    self.project_panel_width + metrics().activity_bar_width
                } else {
                    metrics().activity_bar_width
                },
            ))
            .h_full()
            .flex()
            .flex_shrink_0()
            .min_w(px(0.))
            .flex_col()
            .bg(t.panel_background)
            .border_l_1()
            .child(
                div()
                    .h(m.panel_header_height)
                    .px_3()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child("Axiom AI")
                    .child(
                        div()
                            .flex()
                            .gap_1()
                            .child(
                                div()
                                    .id("ai-new-chat")
                                    .px_1()
                                    .cursor(CursorStyle::PointingHand)
                                    .hover(move |s| s.bg(t.hover))
                                    .tooltip(|_, cx| tooltip("New conversation", cx))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if this.ai_panel_mode == AiPanelMode::Agent {
                                            this.new_agent_conversation(cx);
                                        } else {
                                            this.new_chat(cx);
                                        }
                                    }))
                                    .child("+"),
                            )
                            .child(
                                div()
                                    .id("ai-chat-history")
                                    .px_1()
                                    .cursor(CursorStyle::PointingHand)
                                    .hover(move |s| s.bg(t.hover))
                                    .on_click(
                                        cx.listener(|this, _, _, cx| {
                                            if this.ai_panel_mode == AiPanelMode::Agent {
                                                this.toggle_agent_history(cx);
                                            } else {
                                                this.toggle_chat_history(cx);
                                            }
                                        }),
                                    )
                                    .child(if self.ai_panel_mode == AiPanelMode::Agent {
                                        "Agent history"
                                    } else {
                                        "History"
                                    }),
                            ),
                    ),
            )
            .child(
                div()
                    .h(px(36.))
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_1()
                    .border_b_1()
                    .border_color(t.border_subtle)
                    .child(
                        div()
                            .id("ai-chat-tab")
                            .cursor(CursorStyle::PointingHand)
                            .px_3()
                            .py_1()
                            .rounded(m.border_radius_small)
                            .when(self.ai_panel_mode == AiPanelMode::Chat, |this| {
                                this.bg(t.inactive_selection)
                            })
                            .text_color(t.text_primary)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.switch_ai_panel_mode(AiPanelMode::Chat, cx);
                            }))
                            .child("Chat"),
                    )
                    .child(
                        div()
                            .id("ai-agent-tab")
                            .px_3()
                            .py_1()
                            .rounded(m.border_radius_small)
                            .cursor(CursorStyle::PointingHand)
                            .when(self.ai_panel_mode == AiPanelMode::Agent, |this| {
                                this.bg(t.inactive_selection)
                            })
                            .text_color(t.text_muted)
                            .hover(move |s| s.bg(t.hover).text_color(t.text_primary))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.switch_ai_panel_mode(AiPanelMode::Agent, cx);
                            }))
                            .child("Agent"),
                    )
                    .child(
                        div()
                            .id("ai-memory-mode-tab")
                            .px_3()
                            .py_1()
                            .rounded(m.border_radius_small)
                            .cursor(CursorStyle::PointingHand)
                            .when(self.ai_panel_mode == AiPanelMode::Memory, |this| {
                                this.bg(t.inactive_selection).text_color(t.text_primary)
                            })
                            .when(self.ai_panel_mode != AiPanelMode::Memory, |this| {
                                this.text_color(t.text_muted)
                            })
                            .hover(move |s| s.bg(t.hover).text_color(t.text_primary))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.switch_ai_panel_mode(AiPanelMode::Memory, cx);
                            }))
                            .child("Memory"),
                    ),
            )
            .when(
                self.chat_history_open && self.ai_panel_mode == AiPanelMode::Chat,
                |this| {
                    this.child(
                        div()
                            .id("ai-chat-history-list")
                            .max_h(px(180.))
                            .overflow_y_scroll()
                            .p_2()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .children(self.chat_conversations.iter().map(|chat| {
                                let id = chat.id;
                                let renaming = self.renaming_conversation_id == Some(id);
                                div()
                                    .id(SharedString::from(format!("ai-chat-history-{}", id)))
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .px_2()
                                    .py_1()
                                    .when(id == self.active_chat_id, |this| {
                                        this.bg(t.inactive_selection).text_color(t.text_primary)
                                    })
                                    .hover(move |s| s.bg(t.hover))
                                    .on_click(
                                        cx.listener(move |this, _, _, cx| this.open_chat(id, cx)),
                                    )
                                    .when(renaming, |this| {
                                        this.child(
                                            div()
                                                .id(SharedString::from(format!(
                                                    "ai-chat-rename-{}",
                                                    id
                                                )))
                                                .key_context("SingleLineInput")
                                                .cursor(CursorStyle::IBeam)
                                                .on_action(cx.listener(Self::input_backspace))
                                                .on_action(cx.listener(Self::input_delete))
                                                .on_action(cx.listener(Self::input_left))
                                                .on_action(cx.listener(Self::input_right))
                                                .on_action(cx.listener(Self::input_home))
                                                .on_action(cx.listener(Self::input_end))
                                                .on_action(cx.listener(Self::input_select_all))
                                                .on_action(cx.listener(Self::input_select_left))
                                                .on_action(cx.listener(Self::input_select_right))
                                                .on_action(cx.listener(Self::input_select_home))
                                                .on_action(cx.listener(Self::input_select_end))
                                                .on_action(cx.listener(Self::input_copy))
                                                .on_action(cx.listener(Self::input_cut))
                                                .on_action(cx.listener(Self::input_paste))
                                                .on_action(cx.listener(Self::input_enter))
                                                .on_action(cx.listener(Self::input_escape))
                                                .track_focus(&self.chat_rename_focus)
                                                .overflow_hidden()
                                                .h(px(28.))
                                                .flex_1()
                                                .px_1()
                                                .on_mouse_down(
                                                    MouseButton::Left,
                                                    cx.listener(|this, _, window, cx| {
                                                        cx.stop_propagation();
                                                        this.active_text_input =
                                                            ActiveTextInput::AiHistoryRename;
                                                        window.focus(&this.chat_rename_focus);
                                                        cx.notify();
                                                    }),
                                                )
                                                .child(crate::ui::input_line::render_state(
                                                    cx.entity(),
                                                    &self.chat_rename_input,
                                                )),
                                        )
                                    })
                                    .when(!renaming, |this| this.child(chat.title.clone()))
                                    .when(!renaming, |this| {
                                        this.child(
                                            div()
                                                .id(SharedString::from(format!(
                                                    "ai-chat-rename-button-{}",
                                                    id
                                                )))
                                                .cursor(CursorStyle::PointingHand)
                                                .on_click(cx.listener(
                                                    move |this, _, window, cx| {
                                                        cx.stop_propagation();
                                                        this.begin_chat_rename(id, window, cx)
                                                    },
                                                ))
                                                .child("✎"),
                                        )
                                    })
                                    .child(
                                        div()
                                            .id(SharedString::from(format!(
                                                "ai-chat-delete-{}",
                                                id
                                            )))
                                            .cursor(CursorStyle::PointingHand)
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.delete_chat(id, cx)
                                            }))
                                            .child("×"),
                                    )
                            })),
                    )
                },
            )
            .when(
                self.agent_history_open && self.ai_panel_mode == AiPanelMode::Agent,
                |this| {
                    this.child(
                        div()
                            .id("ai-agent-history-list")
                            .max_h(px(180.))
                            .overflow_y_scroll()
                            .p_2()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .when(self.agent_history_conversations.is_empty(), |this| {
                                this.child(div().px_2().py_1().text_color(t.text_muted).child(
                                    "No saved Agent conversations",
                                ))
                            })
                            .children(self.agent_history_conversations.iter().map(|conversation| {
                                let id = conversation.id.clone();
                                let open_id = id.clone();
                                let active = self.active_agent_conversation_id.as_deref()
                                    == Some(conversation.id.as_str());
                                div()
                                    .id(SharedString::from(format!("ai-agent-history-{}", id)))
                                    .px_2()
                                    .py_1()
                                    .cursor(CursorStyle::PointingHand)
                                    .when(active, |this| {
                                        this.bg(t.inactive_selection).text_color(t.text_primary)
                                    })
                                    .when(!active, |this| this.text_color(t.text_secondary))
                                    .hover(move |style| style.bg(t.hover))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.open_agent_conversation(open_id.clone(), cx)
                                    }))
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_2()
                                            .flex_1()
                                            .min_w_0()
                                            .truncate()
                                            .child(conversation.title.clone())
                                            .child(
                                                div()
                                                    .flex_shrink_0()
                                                    .text_xs()
                                                    .text_color(t.text_muted)
                                                    .child(format_timestamp(conversation.updated_at)),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .id(SharedString::from(format!(
                                                "ai-agent-history-delete-{}",
                                                id
                                            )))
                                            .flex_shrink_0()
                                            .px_2()
                                            .py_1()
                                            .rounded(m.border_radius_small)
                                            .text_color(t.text_muted)
                                            .cursor(CursorStyle::PointingHand)
                                            .hover(move |style| {
                                                style.bg(t.hover).text_color(t.text_primary)
                                            })
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                cx.stop_propagation();
                                                this.delete_agent_conversation(id.clone(), cx)
                                            }))
                                            .child("×"),
                                    )
                            })),
                    )
                },
            )
            .when(self.ai_panel_mode == AiPanelMode::Chat, |this| {
                this.child(self.render_ai_chat_conversation(cx))
            })
            .when(self.ai_panel_mode == AiPanelMode::Agent, |this| {
                this.child(self.render_agent_conversation(cx))
            })
            .when(self.ai_panel_mode == AiPanelMode::Memory, |this| {
                this.child(self.render_memory_panel(cx))
            })
            .when(
                !self.chat_messages.is_empty() && self.ai_panel_mode == AiPanelMode::Chat,
                |this| {
                    this.child(
                        div()
                            .id("ai-scroll-to-latest")
                            .absolute()
                            .bottom(px(92.))
                            .right_3()
                            .w(m.icon_size + px(12.))
                            .h(m.icon_size + px(12.))
                            .rounded(px(999.))
                            .bg(t.elevated_surface)
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor(CursorStyle::PointingHand)
                            .hover(move |s| s.bg(t.hover))
                            .tooltip(|_, cx| tooltip("Scroll to latest", cx))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.jump_chat_to_latest(cx);
                            }))
                            .child(arrow_down_icon(t.accent)),
                    )
                },
            )
            .when(
                matches!(self.chat_request_state, ChatRequestState::Sending),
                |this| {
                    let dots = "•".repeat((self.chat_generating_phase as usize / 2 + 1).min(5));
                    this.child(
                        div()
                            .id("ai-chat-generating-indicator")
                            .absolute()
                            .bottom(px(96.))
                            .left_0()
                            .right_0()
                            .px_2()
                            .py_1()
                            .flex()
                            .justify_center()
                            .text_color(t.text_muted)
                            .child(dots),
                    )
                },
            )
            .when(self.ai_panel_mode != AiPanelMode::Memory, |this| {
                this.child(
                div()
                    .m_3()
                    .p_2()
                    .when(self.ai_panel_mode == AiPanelMode::Chat, |this| {
                        this.h(px(82.))
                    })
                    .when(self.ai_panel_mode == AiPanelMode::Agent, |this| {
                        this.min_h(px(82.)).flex_shrink_0()
                    })
                    .flex()
                    .flex_col()
                    .justify_between()
                    .rounded(m.border_radius_medium)
                    .bg(t.editor_background)
                    .border_1()
                    .border_color(t.border_subtle)
                    .when(context_is_active, |this| {
                        this.child(
                            div()
                                .id("ai-agent-context")
                                .w_full()
                                .min_w_0()
                                .overflow_hidden()
                                .px_2()
                                .py_1()
                                .cursor(CursorStyle::PointingHand)
                                .text_color(t.accent)
                                .hover(move |s| s.bg(t.hover))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.chat_context_sources.clear();
                                    this.chat_context_feedback = None;
                                    cx.notify();
                                }))
                                .when_some(context_full_label.clone(), |this, label| {
                                    this.tooltip(move |_, cx| tooltip(label.clone(), cx))
                                })
                                .child(SharedString::from(format!(
                                    "[ {} × ]",
                                    context_label.clone().unwrap_or_default()
                                ))),
                        )
                    })
                    .child(
                        div()
                            .id("ai-composer-input")
                            .key_context("SingleLineInput")
                            .cursor(CursorStyle::IBeam)
                            .on_action(cx.listener(Self::input_backspace))
                            .on_action(cx.listener(Self::input_delete))
                            .on_action(cx.listener(Self::input_left))
                            .on_action(cx.listener(Self::input_right))
                            .on_action(cx.listener(Self::input_home))
                            .on_action(cx.listener(Self::input_end))
                            .on_action(cx.listener(Self::input_select_all))
                            .on_action(cx.listener(Self::input_select_left))
                            .on_action(cx.listener(Self::input_select_right))
                            .on_action(cx.listener(Self::input_select_home))
                            .on_action(cx.listener(Self::input_select_end))
                            .on_action(cx.listener(Self::input_copy))
                            .on_action(cx.listener(Self::input_cut))
                            .on_action(cx.listener(Self::input_paste))
                            .on_action(cx.listener(Self::input_enter))
                            .track_focus(&self.ai_composer_focus)
                            .overflow_hidden()
                            .h(px(34.))
                            .w_full()
                            .px_2()
                            .flex()
                            .items_center()
                            .bg(t.editor_background)
                            .border_1()
                            .border_color(if self.ai_composer_active {
                                t.accent
                            } else {
                                t.border_subtle
                            })
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                                    this.ai_composer_active = true;
                                    this.active_text_input = ActiveTextInput::AiComposer;
                                    this.provider_base_url_active = false;
                                    this.provider_base_url.active = false;
                                    this.provider_api_key.active = false;
                                    this.provider_model_input.active = false;
                                    window.focus(&this.ai_composer_focus);
                                    let byte = this.ai_composer_geometry.hit_test(event.position.x);
                                    let caret = byte_to_utf16_offset(&this.ai_composer_text, byte);
                                    if event.click_count >= 2 {
                                        let end = this.ai_composer_text.encode_utf16().count();
                                        this.ai_composer_selection = UTF16Selection {
                                            range: 0..end,
                                            reversed: false,
                                        };
                                        this.ai_composer_dragging = false;
                                    } else {
                                        this.ai_composer_selection = UTF16Selection {
                                            range: caret..caret,
                                            reversed: false,
                                        };
                                        this.ai_composer_dragging = true;
                                    }
                                    this.modal_caret_visible = true;
                                    this.modal_caret_activity = Instant::now();
                                    this.modal_caret_toggle = this.modal_caret_activity;
                                    cx.notify();
                                }),
                            )
                            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                                if this.active_text_input == ActiveTextInput::AiComposer
                                    && this.ai_composer_dragging
                                    && event.dragging()
                                {
                                    let byte = this.ai_composer_geometry.hit_test(event.position.x);
                                    let head = byte_to_utf16_offset(&this.ai_composer_text, byte);
                                    this.ai_composer_selection.range.end = head;
                                    this.ai_composer_selection.reversed =
                                        this.ai_composer_selection.range.start > head;
                                    cx.notify();
                                }
                            }))
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.ai_composer_dragging = false;
                                    cx.notify();
                                }),
                            )
                            .child(crate::ui::input_line::render(
                                cx.entity(),
                                self.ai_composer_focus.clone(),
                                if self.ai_composer_text.is_empty() {
                                    "Ask Axiom...".into()
                                } else {
                                    self.ai_composer_text.clone()
                                },
                                if self.ai_composer_active {
                                    utf16_to_byte_offset(
                                        &self.ai_composer_text,
                                        self.ai_composer_selection
                                            .range
                                            .start
                                            .min(self.ai_composer_selection.range.end),
                                    )
                                        ..utf16_to_byte_offset(
                                            &self.ai_composer_text,
                                            self.ai_composer_selection
                                                .range
                                                .start
                                                .max(self.ai_composer_selection.range.end),
                                        )
                                } else {
                                    0..0
                                },
                                utf16_to_byte_offset(
                                    &self.ai_composer_text,
                                    self.ai_composer_selection.range.end,
                                ),
                                self.modal_caret_visible,
                                self.ai_composer_geometry.clone(),
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .w_full()
                            .min_w_0()
                            .text_color(t.text_muted)
                            .child(
                                div()
                                    .flex()
                                    .w_full()
                                    .min_w_0()
                                    .items_center()
                                    .justify_between()
                                    .gap_1()
                                    .when(!context_is_active, |this| {
                                        this.child(
                                            div()
                                                .id("ai-context-button")
                                                .flex_shrink_0()
                                                .px_2()
                                                .py_1()
                                                .cursor(CursorStyle::PointingHand)
                                                .hover(move |s| s.bg(t.hover))
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.capture_active_editor_context(cx);
                                                }))
                                                .child(SharedString::from(
                                                    self.chat_context_feedback
                                                        .clone()
                                                        .unwrap_or_else(|| "+ Context".into()),
                                                )),
                                        )
                                    })
                                    .when(self.selected_model_supports_thinking(), |this| {
                                        this.child(
                                            div()
                                                .id("ai-thinking-toggle")
                                                .flex_shrink_0()
                                                .px_1()
                                                .cursor(CursorStyle::PointingHand)
                                                .text_color(if self.chat_thinking_enabled {
                                                    t.accent
                                                } else {
                                                    t.text_muted
                                                })
                                                .hover(move |s| s.bg(t.hover))
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.chat_thinking_enabled =
                                                        !this.chat_thinking_enabled;
                                                    if let Some(provider) =
                                                        this.active_provider.clone()
                                                    {
                                                        this.thinking_preferences.insert(
                                                            (provider, this.model_label.clone()),
                                                            this.chat_thinking_enabled,
                                                        );
                                                        this.persist_ui_settings();
                                                    }
                                                    cx.notify();
                                                }))
                                                .child(if self.chat_thinking_enabled {
                                                    "Thinking: On"
                                                } else {
                                                    "Thinking: Off"
                                                }),
                                        )
                                    })
                                    .when(
                                        self.selected_thinking_capability().levels.len() > 1,
                                        |this| {
                                            let current = self
                                                .selected_thinking_level()
                                                .map(ThinkingLevel::label)
                                                .unwrap_or("Level");
                                            this.child(
                                                div()
                                                    .id("ai-thinking-level-button")
                                                    .flex_shrink_0()
                                                    .px_1()
                                                    .cursor(CursorStyle::PointingHand)
                                                    .text_color(if self.chat_thinking_enabled {
                                                        t.text_primary
                                                    } else {
                                                        t.text_muted
                                                    })
                                                    .hover(move |s| s.bg(t.hover))
                                                    .on_mouse_down(
                                                        MouseButton::Left,
                                                        cx.listener(|this, _, _, cx| {
                                                            cx.stop_propagation();
                                                            this.thinking_level_picker_open =
                                                                !this.thinking_level_picker_open;
                                                            cx.notify();
                                                        }),
                                                    )
                                                    .child(format!("{current} ▾")),
                                            )
                                        },
                                    )
                                    .child(
                                        div()
                                            .id("ai-model-button")
                                            .flex_1()
                                            .min_w_0()
                                            .truncate()
                                            .px_1()
                                            .hover(move |s| s.bg(t.hover))
                                            .tooltip({
                                                let model_label = self.model_label.clone();
                                                move |_, cx| tooltip(model_label.clone(), cx)
                                            })
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.toggle_model_picker(cx)
                                            }))
                                            .child(self.model_label.clone()),
                                    )
                                    .child(
                                        div()
                                            .id("ai-send-button")
                                            .flex_shrink_0()
                                            .px_2()
                                            .rounded(m.border_radius_small)
                                            .when(self.is_ai_generation_active(), |this| {
                                                this.cursor(CursorStyle::PointingHand)
                                                    .text_color(t.accent)
                                                    .hover(move |s| s.bg(t.hover))
                                                    .tooltip(|_, cx| tooltip("Stop generating", cx))
                                                    .on_click(cx.listener(|this, _, _, cx| {
                                                        this.stop_ai_generation(cx)
                                                    }))
                                            })
                                            .when(
                                                !self.ai_composer_text.trim().is_empty()
                                                    && !self.is_ai_generation_active(),
                                                |this| {
                                                    this.cursor(CursorStyle::PointingHand)
                                                        .text_color(t.accent)
                                                        .hover(move |s| s.bg(t.hover))
                                                        .on_click(cx.listener(|this, _, _, cx| {
                                                            this.send_ai_chat_message(cx)
                                                        }))
                                                },
                                            )
                                            .when(self.is_ai_generation_active(), |this| {
                                                this.child(stop_icon(t.accent))
                                            })
                                            .when(!self.is_ai_generation_active(), |this| {
                                                this.child("↑")
                                            }),
                                    ),
                            ),
                    ),
                )
            })
            .when(self.model_picker_open, |this| {
                this.child(self.render_model_picker(cx))
            })
            .when(self.thinking_level_picker_open, |this| {
                this.child(self.render_thinking_level_picker(cx))
            })
            .child(
                div()
                    .id("ai-panel-resize-handle")
                    .absolute()
                    .left(px(0.))
                    .top(px(0.))
                    .bottom(px(0.))
                    .w(px(6.))
                    .cursor(CursorStyle::ResizeLeftRight)
                    .bg(t.border_subtle)
                    .hover(|this| this.bg(t.accent))
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::ai_panel_resize_start))
                    .child(gpui::canvas(
                        |_, _, _| (),
                        move |_, _, window, app| {
                            if !workspace.read(app).ai_panel_resizing {
                                return;
                            }
                            let resize_workspace = workspace.clone();
                            window.on_mouse_event(move |event: &gpui::MouseMoveEvent, phase, window, app| {
                                if phase == gpui::DispatchPhase::Capture && event.dragging() {
                                    let _ = resize_workspace.update(app, |this, cx| {
                                        this.ai_panel_resize_move(event, window, cx);
                                    });
                                }
                            });
                            let resize_workspace = workspace.clone();
                            window.on_mouse_event(move |event: &MouseUpEvent, phase, window, app| {
                                if phase == gpui::DispatchPhase::Capture && event.button == MouseButton::Left {
                                    let _ = resize_workspace.update(app, |this, cx| {
                                        this.ai_panel_resize_end(event, window, cx);
                                    });
                                }
                            });
                        },
                    )),
            )
    }

}
