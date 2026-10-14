use super::*;
use super::EditorView;

impl EditorView {
    pub(super) fn maybe_trigger_completion(&self, text: &str) {
        let (Some(lsp), Some(uri)) = (self.lsp.as_ref(), self.lsp_uri.as_ref()) else {
            return;
        };
        let tail = &text[..self.document.cursor_offset().min(text.len())];
        if tail.ends_with("->")
            || tail.ends_with("::")
            || tail.ends_with("new ")
            || tail.ends_with("extends ")
            || tail.ends_with("implements ")
            || tail.ends_with("use ")
        {
            let position = PositionCodec::offset_to_position(
                text,
                self.document.cursor_offset(),
                lsp.encoding(),
            );
            lsp.request_completion(uri.clone(), position);
        }
    }


    pub(super) fn complete(&mut self, _: &Complete, _: &mut Window, cx: &mut Context<Self>) {
        // Keep manual completion on the same cheap, metadata-only gate used
        // by the automatic after-edit path.
        if !self.is_php_completion_context() {
            self.completions.clear();
            cx.notify();
            return;
        }
        self.completions.clear();
        let native = self.native_completions();
        if let (Some(lsp), Some(uri), Some(position)) =
            (&self.lsp, &self.lsp_uri, self.lsp_position())
        {
            lsp.request_completion(uri.clone(), position);
        }
        if !native.items.is_empty() {
            self.set_completions(native.items, cx);
        } else if self.lsp.is_none() {
            self.status = Some("Completion unavailable (no PHP index or language server)".into());
            cx.notify();
        }
    }

    pub(super) fn native_completions(&self) -> NativeCompletionBatch {
        let Some(syntax) = self.syntax.as_ref() else {
            return NativeCompletionBatch {
                items: Vec::new(),
                new_prefix: None,
            };
        };
        self.native_completions_for_text(syntax.text())
    }

    pub(super) fn native_completions_for_text(&self, text: &str) -> NativeCompletionBatch {
        let _stage = UiStageGuard::new(UI_STAGE_COMPLETION);
        let started = Instant::now();
        let result = self.native_completions_impl(text);
        if debug_ui_stall_enabled() && started.elapsed().as_micros() >= 3_000 {
            tracing::info!(target: "axiom.ui_stall",
                candidates = result.items.len(),
                total_us = started.elapsed().as_micros(),
                "[UI COMPLETION]"
            );
        }
        result
    }

    pub(super) fn native_completions_impl(&self, text: &str) -> NativeCompletionBatch {
        let cursor = self.document.cursor_offset().min(text.len());
        if let Some(tag_range) = phpdoc_tag_token(text, cursor) {
            let prefix = &text[tag_range.clone()];
            let target = self.phpdoc_completion_target(text, cursor);
            let allowed = self
                .phpdoc_completion_tags(text, cursor)
                .unwrap_or(&PHPDOC_TAGS);
            let present = phpdoc_tags_before_cursor(text, cursor);
            let mut items = PHPDOC_TAGS
                .iter()
                .filter(|tag| allowed.contains(tag))
                .filter(|tag| {
                    !phpdoc_singleton(*tag) || !present.iter().any(|present| present == *tag)
                })
                .filter(|tag| {
                    !matches!(**tag, "@param" | "@return")
                        || !phpdoc_semantic_tag_available(target.as_ref(), **tag, &present)
                })
                .filter(|tag| starts_with_ascii_case_insensitive(*tag, prefix))
                .map(|tag| CompletionItem {
                    label: (*tag).to_owned(),
                    detail: Some("PHPDoc tag".to_owned()),
                    kind: Some(CompletionItemKind::KEYWORD),
                    insert_text: Some(format!("{tag} ")),
                    ..Default::default()
                })
                .collect::<Vec<_>>();
            if let Some(DocblockTarget::Callable {
                parameters,
                return_type,
                ..
            }) = target
            {
                let documented_params = phpdoc_documented_param_names(text, cursor);
                if allowed.contains(&"@param") {
                    for parameter in parameters {
                        if parameter.declared_type.is_none()
                            || documented_params.contains(&parameter.name)
                        {
                            continue;
                        }
                        let label = format!(
                            "@param {} {}",
                            parameter.declared_type.as_deref().unwrap_or_default(),
                            parameter.name
                        );
                        if starts_with_ascii_case_insensitive("@param", prefix) {
                            items.push(CompletionItem {
                                label: label.clone(),
                                detail: Some("PHPDoc parameter from signature".to_owned()),
                                kind: Some(CompletionItemKind::KEYWORD),
                                insert_text: Some(format!("{label} ")),
                                ..Default::default()
                            });
                        }
                    }
                }
                if allowed.contains(&"@return")
                    && !present.iter().any(|tag| *tag == "@return")
                    && let Some(return_type) = return_type
                    && return_type.trim() != "void"
                {
                    let return_type = normalize_phpdoc_return_type(&return_type);
                    let label = format!("@return {return_type}");
                    if starts_with_ascii_case_insensitive("@return", prefix) {
                        items.push(CompletionItem {
                            label: label.clone(),
                            detail: Some("PHPDoc return type from signature".to_owned()),
                            kind: Some(CompletionItemKind::KEYWORD),
                            insert_text: Some(format!("{label} ")),
                            ..Default::default()
                        });
                    }
                }
                let throws = self.explicit_throws_for_docblock(text, cursor);
                if allowed.contains(&"@throws") && !throws.is_empty() {
                    let documented = phpdoc_documented_throws(text, cursor);
                    items.retain(|item| item.label != "@throws");
                    for exception in throws {
                        if documented.iter().any(|value| value == &exception)
                            || !starts_with_ascii_case_insensitive("@throws", prefix)
                        {
                            continue;
                        }
                        let label = format!("@throws {exception}");
                        items.push(CompletionItem {
                            label: label.clone(),
                            detail: Some("PHPDoc exception from explicit throw".to_owned()),
                            kind: Some(CompletionItemKind::KEYWORD),
                            insert_text: Some(format!("{label} ")),
                            ..Default::default()
                        });
                    }
                }
            }
            return NativeCompletionBatch {
                items,
                new_prefix: Some(tag_range),
            };
        }
        if self.target_php_version.supports_property_hooks()
            && let Some((range, items)) = php_property_hook_completions(text, cursor)
        {
            return NativeCompletionBatch {
                items,
                new_prefix: Some(range),
            };
        }
        if let Some((_, items)) = php_class_member_completions(text, cursor) {
            return NativeCompletionBatch {
                items,
                new_prefix: None,
            };
        }
        if let Some((template_range, template_items)) = php_live_template_completions(
            text,
            cursor,
            self.target_php_version,
        )
        {
            return NativeCompletionBatch {
                items: template_items,
                new_prefix: Some(template_range),
            };
        }
        let before = &text[..cursor];
        let member_operator = before
            .char_indices()
            .rev()
            .find_map(|(index, ch)| (ch == '>' || ch == ':').then_some((index, ch)))
            .and_then(|(index, ch)| {
                if ch == '>' && before[..index].ends_with('-') {
                    let operator_start = index.saturating_sub(1);
                    let suffix = &before[index + 1..];
                    suffix
                        .chars()
                        .all(|character| character.is_alphanumeric() || character == '_')
                        .then_some((operator_start, false))
                } else if ch == ':' && before[..index].ends_with(':') {
                    let operator_start = index.saturating_sub(1);
                    let suffix = &before[index + 1..];
                    suffix
                        .chars()
                        .all(|character| character.is_alphanumeric() || character == '_')
                        .then_some((operator_start, true))
                } else {
                    None
                }
            });
        let type_site = php_type_completion_site(text, cursor);
        let start = type_site
            .as_ref()
            .map(|site| site.range.start)
            .unwrap_or_else(|| {
                text[..cursor]
                    .char_indices()
                    .rev()
                    .take_while(|(_, ch)| ch.is_alphanumeric() || matches!(ch, '_' | '$'))
                    .last()
                    .map_or(cursor, |(index, _)| index)
            });
        let prefix = &text[start..cursor];
        let preceded_by_new = type_site.is_none() && before[..start].trim_end().ends_with("new");
        let type_context = type_site
            .as_ref()
            .map(|site| site.context)
            .unwrap_or_else(|| type_completion_context(before, start, preceded_by_new));
        let pipe_rhs_context = self.target_php_version.supports_pipe_operator()
            && self.syntax.as_ref().is_some_and(|syntax| {
                syntax
                    .pipe_operator_ranges()
                    .into_iter()
                    .filter(|range| range.end <= cursor)
                    .max_by_key(|range| range.end)
                    .is_some_and(|range| {
                        !text[range.end..cursor].contains([';', '{', '}'])
                    })
            });
        if prefix.starts_with('$') {
            return NativeCompletionBatch {
                items: if before[..start].ends_with("::") || before[..start].ends_with("->") {
                    Vec::new()
                } else {
                    self.local_variable_completions(&text[..cursor], prefix)
                },
                new_prefix: None,
            };
        }
        let empty_prefix_context = before.ends_with("new ")
            || before.ends_with("extends ")
            || before.ends_with("implements ")
            || before.ends_with("use ")
            || type_site.is_some()
            || pipe_rhs_context;
        if prefix.is_empty() && member_operator.is_none() && !empty_prefix_context {
            return NativeCompletionBatch {
                items: Vec::new(),
                new_prefix: None,
            };
        }
        // A class name by itself is also a static-access completion context.
        // This lets `CustomRuntime` expand directly to
        // `CustomRuntime::hello($name, $age)` without requiring `::` first.
        if member_operator.is_none()
            && !prefix.is_empty()
            && !empty_prefix_context
            && !preceded_by_new
            && type_site.is_none()
        {
            let class_exists = self.runtime_symbols.as_ref().is_some_and(|index| {
                index.find_class(prefix).is_some()
                    || index.find_class_by_short_name(prefix).is_some()
            }) || self
                .project_symbols
                .as_ref()
                .and_then(|index| index.try_read().ok())
                .is_some_and(|index| index.find_class(prefix).is_some());
            if class_exists {
                let class_fqn = self.resolve_native_type(prefix, before);
                let mut items = Vec::new();
                if let (Some(runtime), Some(class_fqn)) =
                    (&self.runtime_symbols, class_fqn.as_ref())
                {
                    let runtime_fqn = runtime
                        .find_class(class_fqn)
                        .or_else(|| runtime.find_class_by_short_name(prefix))
                        .map(|symbol| symbol.fqn.clone())
                        .unwrap_or_else(|| class_fqn.clone());
                    items.extend(
                        runtime
                            .methods_of(&runtime_fqn)
                            .filter(|symbol| symbol.is_static)
                            .map(|symbol| {
                                let call = runtime_call_insert_text(symbol)
                                    .unwrap_or_else(|| format!("{}()", symbol.name));
                                CompletionItem {
                                    label: symbol.name.clone(),
                                    detail: Some(runtime_signature_detail(symbol)),
                                    kind: Some(CompletionItemKind::METHOD),
                                    insert_text: Some(format!("{prefix}::{call}")),
                                    ..Default::default()
                                }
                            }),
                    );
                }
                if let (Some(project), Some(class_fqn)) =
                    (&self.project_symbols, class_fqn.as_ref())
                    && let Ok(project) = project.try_read()
                {
                    items.extend(
                        project
                            .find_methods(class_fqn)
                            .into_iter()
                            .filter(|symbol| {
                                symbol.modifiers.iter().any(|modifier| modifier == "static")
                            })
                            .map(|symbol| {
                                let detail = project_method_detail(symbol);
                                let signature = detail
                                    .find('(')
                                    .and_then(|open| {
                                        detail.rfind(')').map(|close| &detail[open..=close])
                                    })
                                    .unwrap_or("()")
                                    .to_owned();
                                CompletionItem {
                                    label: symbol.name.clone(),
                                    detail: Some(detail),
                                    kind: Some(CompletionItemKind::METHOD),
                                    insert_text: Some(format!(
                                        "{prefix}::{}{signature}",
                                        symbol.name
                                    )),
                                    ..Default::default()
                                }
                            }),
                    );
                }
                if items.is_empty() {
                    items.push(CompletionItem {
                        label: "class".to_owned(),
                        detail: Some("Class name • Runtime".to_owned()),
                        kind: Some(CompletionItemKind::KEYWORD),
                        insert_text: Some(format!("{prefix}::class")),
                        ..Default::default()
                    });
                }
                return NativeCompletionBatch {
                    items: items.into_iter().take(40).collect(),
                    new_prefix: None,
                };
            }
        }
        if let Some((operator_start, is_static)) = member_operator {
            let owner_end = operator_start;
            let (owner_start, owner_expression) = extract_owner_expression(&text, owner_end);
            let owner = owner_expression.trim_start_matches('$');
            let mut snapshot_receiver_type = None;
            let mut snapshot_binding_checked = false;

            // For direct instance-member completion, prefer the resident
            // semantic snapshot. It already knows the binding type and walks
            // inherited/trait owners without touching the filesystem or
            // scanning the project index. The legacy path below remains only
            // for contexts that cannot be represented by a snapshot binding
            // (for example an unresolved chained expression).
            if !is_static && let Some(engine) = &self.semantic_engine {
                let snapshot = engine.snapshot();
                let file_key = PersistentFileKey::workspace_lexical(&self.file_path);
                if let Some(scope) = snapshot.scope_id_at(&file_key, cursor.saturating_sub(1)) {
                    snapshot_binding_checked = owner_expression.starts_with('$');
                    if let Some(binding) =
                        snapshot.lookup_binding_at(scope, &owner_expression, owner_end)
                    {
                        snapshot_receiver_type = binding.declared_type.as_ref().and_then(|ty| {
                            if let DeclaredType::Named { resolved, .. } = ty {
                                Some(resolved.clone())
                            } else {
                                None
                            }
                        });
                        let ids = snapshot
                            .member_resolver()
                            .completion_methods_for_binding_at(
                                scope,
                                &owner_expression,
                                prefix,
                                owner_end,
                            );
                        let members = ids
                            .into_iter()
                            .filter_map(|id| snapshot.symbol(id))
                            .map(|symbol| CompletionItem {
                                label: symbol.name.clone(),
                                detail: Some(format!(
                                    "{}{}{} • Project",
                                    symbol.name,
                                    symbol.parameters.clone().unwrap_or_else(|| "()".to_owned()),
                                    symbol
                                        .return_type
                                        .as_deref()
                                        .map(|value| format!(": {value}"))
                                        .unwrap_or_default()
                                )),
                                kind: Some(CompletionItemKind::METHOD),
                                ..Default::default()
                            })
                            .take(40)
                            .collect::<Vec<_>>();
                        if !members.is_empty() {
                            return NativeCompletionBatch {
                                items: members,
                                new_prefix: None,
                            };
                        }
                    }
                }
                if snapshot_receiver_type.is_none()
                    && !owner_expression.is_empty()
                    && let Some(syntax) = &self.syntax
                    && let Some(scope) =
                        snapshot.scope_id_at(&file_key, owner_end.saturating_sub(1))
                    && let Some(mut node) = syntax.tree().root_node().descendant_for_byte_range(
                        owner_end.saturating_sub(1),
                        owner_end,
                    )
                {
                    let mut expression = None;
                    loop {
                        if node.start_byte() >= owner_start
                            && node.end_byte() == owner_end
                            && matches!(
                                node.kind(),
                                "object_creation_expression"
                                    | "member_call_expression"
                                    | "nullsafe_member_call_expression"
                                    | "parenthesized_expression"
                                    | "function_call_expression"
                            )
                        {
                            expression = Some(node);
                        }
                        let Some(parent) = node.parent() else { break };
                        if parent.start_byte() < owner_start || parent.end_byte() != owner_end {
                            break;
                        }
                        node = parent;
                    }
                    if let Some(expression) = expression {
                        snapshot_receiver_type = axiom_index::ExpressionResolver::new(
                            &snapshot,
                            scope,
                        )
                        .infer_ast_expression_type(expression, &text)
                        .and_then(|ty| match ty {
                            DeclaredType::Named { resolved, .. } => Some(resolved),
                            _ => None,
                        });
                    }
                }
            }

            if snapshot_binding_checked && snapshot_receiver_type.is_none() {
                return NativeCompletionBatch {
                    items: Vec::new(),
                    new_prefix: None,
                };
            }

            if let Some(class_fqn) = snapshot_receiver_type
                .or_else(|| self.resolve_receiver_type(&owner_expression, &text[..owner_end]))
            {
                let mut members = Vec::new();
                if let Some(index) = &self.project_symbols
                    && let Ok(index) = index.try_read()
                {
                    members.extend(
                        index
                            .find_methods(&class_fqn)
                            .into_iter()
                            .filter(|symbol| {
                                symbol.name.starts_with(prefix)
                                    && (is_static == symbol.modifiers.iter().any(|m| m == "static"))
                                    && (is_static
                                        || symbol.visibility != axiom_index::Visibility::Private)
                            })
                            .map(|symbol| CompletionItem {
                                label: symbol.name.clone(),
                                detail: Some(project_method_detail(symbol)),
                                kind: Some(CompletionItemKind::METHOD),
                                ..Default::default()
                            }),
                    );
                }
                if let Some(index) = &self.vendor_symbols
                    && let Ok(index) = index.try_read()
                {
                    members.extend(
                        index
                            .cached_symbols(&class_fqn)
                            .into_iter()
                            .filter(|symbol| {
                                symbol.name.starts_with(prefix)
                                    && (is_static == symbol.modifiers.iter().any(|m| m == "static"))
                            })
                            .map(|symbol| CompletionItem {
                                label: symbol.name,
                                detail: symbol.parameters.clone(),
                                kind: Some(CompletionItemKind::METHOD),
                                ..Default::default()
                            }),
                    );
                }
                if let Some(index) = &self.runtime_symbols {
                    let runtime_class_fqn = index
                        .find_class(&class_fqn)
                        .map(|symbol| symbol.fqn.clone())
                        .or_else(|| {
                            index
                                .find_class_by_short_name(owner)
                                .map(|symbol| symbol.fqn.clone())
                        })
                        .unwrap_or_else(|| class_fqn.clone());
                    members.extend(
                        index
                            .methods_of(&runtime_class_fqn)
                            .filter(|symbol| {
                                symbol.name.starts_with(prefix)
                                    && symbol.is_static == is_static
                                    && (is_static || !symbol.name.starts_with('_'))
                            })
                            .map(|symbol| CompletionItem {
                                label: symbol.name.clone(),
                                detail: Some(runtime_signature_detail(symbol)),
                                kind: Some(CompletionItemKind::METHOD),
                                insert_text: runtime_call_insert_text(symbol),
                                ..Default::default()
                            }),
                    );
                    members.extend(
                        index
                            .members_of(&runtime_class_fqn)
                            .iter()
                            .filter(|symbol| {
                                symbol.kind == RuntimeKind::Property
                                    && symbol.name.starts_with(prefix)
                            })
                            .map(|symbol| CompletionItem {
                                label: symbol.name.clone(),
                                detail: Some(format!("{} • PHP Runtime", symbol.name)),
                                kind: Some(CompletionItemKind::FIELD),
                                ..Default::default()
                            }),
                    );
                }
                if is_static && members.is_empty() && prefix.is_empty() {
                    members.push(CompletionItem {
                        label: "class".to_owned(),
                        detail: Some("Class name • Runtime".to_owned()),
                        kind: Some(CompletionItemKind::KEYWORD),
                        insert_text: Some("class".to_owned()),
                        ..Default::default()
                    });
                }
                let mut seen = std::collections::HashSet::new();
                members.retain(|item| seen.insert(item.label.to_ascii_lowercase()));
                return NativeCompletionBatch {
                    items: members.into_iter().take(40).collect(),
                    new_prefix: None,
                };
            }
        }
        let mut items = Vec::new();
        if let Some(site) = &type_site {
            items.extend(
                PHP_NATIVE_TYPES
                    .iter()
                    .filter(|type_name| {
                        starts_with_ascii_case_insensitive(*type_name, prefix)
                            && php_native_type_allowed(site, *type_name)
                    })
                    .map(|type_name| CompletionItem {
                        label: (*type_name).to_owned(),
                        detail: Some("PHP type".to_owned()),
                        kind: Some(CompletionItemKind::KEYWORD),
                        insert_text: Some((*type_name).to_owned()),
                        ..Default::default()
                    }),
            );
        }
        let runtime_type_context = type_site.is_some()
            || type_context != TypeCompletionContext::GenericType;
        items.extend(
            self.runtime_symbols
                .as_ref()
                .map(|index| {
                    let matches = if pipe_rhs_context {
                        index
                            .search_prefix(prefix)
                            .into_iter()
                            .filter(|symbol| symbol.kind == RuntimeKind::Function)
                            .collect()
                    } else if runtime_type_context {
                        index.search_type_prefix(prefix)
                    } else {
                        index.search_prefix(prefix)
                    };
                    matches
                        .into_iter()
                        .filter(|symbol| {
                            type_site
                                .as_ref()
                                .map(|site| runtime_type_kind_for_site(site, symbol.kind))
                                .unwrap_or_else(|| {
                                    if type_context != TypeCompletionContext::GenericType {
                                        runtime_type_kind_allowed(type_context, symbol.kind)
                                    } else {
                                        runtime_global_kind_allowed(
                                            before,
                                            text[cursor..].starts_with("::"),
                                            prefix,
                                            start,
                                            preceded_by_new,
                                            member_operator.is_some(),
                                            symbol.kind,
                                        )
                                    }
                                })
                        })
                        .take(40)
                        .map(|symbol| {
                            let import = matches!(
                                symbol.kind,
                                RuntimeKind::Class
                                    | RuntimeKind::Interface
                                    | RuntimeKind::Trait
                                    | RuntimeKind::Enum
                            )
                            .then(|| {
                                self.composer_import_edit(
                                    text,
                                    &symbol.fqn,
                                    self.document.cursor_offset(),
                                )
                            })
                            .flatten();
                            CompletionItem {
                                label: symbol.name.clone(),
                                detail: Some(
                                    if matches!(
                                        symbol.kind,
                                        RuntimeKind::Function | RuntimeKind::Method
                                    ) {
                                        runtime_signature_detail(symbol)
                                    } else {
                                        format!("{:?} • PHP Runtime", symbol.kind)
                                    },
                                ),
                                kind: Some(match symbol.kind {
                                    RuntimeKind::Function => CompletionItemKind::FUNCTION,
                                    RuntimeKind::Class => CompletionItemKind::CLASS,
                                    RuntimeKind::Interface => CompletionItemKind::INTERFACE,
                                    RuntimeKind::Trait => CompletionItemKind::CLASS,
                                    RuntimeKind::Enum => CompletionItemKind::ENUM,
                                    _ => CompletionItemKind::VALUE,
                                }),
                                insert_text: if pipe_rhs_context {
                                    Some(format!("{}(...) ", symbol.name).trim().to_owned())
                                } else {
                                    runtime_call_insert_text(symbol)
                                },
                                additional_text_edits: import.map(|edit| vec![edit]),
                                ..Default::default()
                            }
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default(),
        );
        let completion_search_started = Instant::now();
        let mut completion_search_us = 0u128;
        let mut completion_import_us = 0u128;
        let mut completion_symbols_total = 0usize;
        let mut completion_matches = 0usize;
        let mut completion_index_unavailable = false;
        if let Some(index) = &self.project_symbols {
            if let Ok(index) = index.try_read() {
                completion_symbols_total = index.symbols().len();
                let search_started = Instant::now();
                let project_matches = index.search_prefix(prefix);
                completion_search_us = search_started.elapsed().as_micros();
                completion_matches = project_matches.len();
                items.extend(
                    project_matches
                        .into_iter()
                        .filter(|symbol| {
                            if pipe_rhs_context {
                                symbol.kind == ProjectSymbolKind::Function
                            } else {
                                type_site
                                    .as_ref()
                                    .map(|site| project_type_kind_for_site(site, symbol.kind))
                                    .unwrap_or_else(|| {
                                        type_kind_allowed(type_context, symbol.kind)
                                    })
                            }
                        })
                        .map(|symbol| {
                            let import_started = Instant::now();
                            let import = matches!(
                                symbol.kind,
                                ProjectSymbolKind::Class
                                    | ProjectSymbolKind::Interface
                                    | ProjectSymbolKind::Trait
                                    | ProjectSymbolKind::Enum
                            )
                            .then(|| {
                                self.composer_import_edit(
                                    text,
                                    &symbol.fully_qualified_name,
                                    self.document.cursor_offset(),
                                )
                            })
                            .flatten();
                            completion_import_us += import_started.elapsed().as_micros();
                            CompletionItem {
                                label: symbol.name.clone(),
                                detail: Some(format!("{} • Project", symbol.fully_qualified_name)),
                                kind: Some(match symbol.kind {
                                    ProjectSymbolKind::Function => CompletionItemKind::FUNCTION,
                                    ProjectSymbolKind::Method => CompletionItemKind::METHOD,
                                    ProjectSymbolKind::Class => CompletionItemKind::CLASS,
                                    ProjectSymbolKind::Interface => CompletionItemKind::INTERFACE,
                                    ProjectSymbolKind::Trait => CompletionItemKind::CLASS,
                                    ProjectSymbolKind::Enum => CompletionItemKind::ENUM,
                                    _ => CompletionItemKind::VALUE,
                                }),
                                insert_text: if pipe_rhs_context
                                    && symbol.kind == ProjectSymbolKind::Function
                                {
                                    Some(format!("{}(...) ", symbol.name).trim().to_owned())
                                } else {
                                    (current_namespace(text).is_empty()
                                        && matches!(
                                            symbol.kind,
                                            ProjectSymbolKind::Class
                                                | ProjectSymbolKind::Interface
                                                | ProjectSymbolKind::Trait
                                                | ProjectSymbolKind::Enum
                                        ))
                                    .then(|| format!("\\{}", symbol.fully_qualified_name))
                                },
                                additional_text_edits: import.map(|edit| vec![edit]),
                                ..Default::default()
                            }
                        }),
                );
            } else {
                completion_index_unavailable = true;
            }
        }
        let completion_inner_us = completion_search_started.elapsed().as_micros();
        if debug_ui_stall_enabled() && completion_index_unavailable {
            tracing::info!(target: "axiom.ui_stall", index_unavailable = true, "[UI COMPLETION SEARCH]");
        } else if debug_ui_stall_enabled() && completion_inner_us >= 3_000 {
            tracing::info!(target: "axiom.ui_stall",
                prefix_len = prefix.len(),
                symbols_total = completion_symbols_total,
                matches = completion_matches,
                search_prefix_us = completion_search_us,
                import_edits_us = completion_import_us,
                total_inner_us = completion_inner_us,
                "[UI COMPLETION SEARCH]"
            );
        }
        if let Some(index) = &self.vendor_symbols
            && let Ok(index) = index.try_read()
        {
            if matches!(
                type_context,
                TypeCompletionContext::ClassExtends
                    | TypeCompletionContext::ClassImplements
                    | TypeCompletionContext::InterfaceExtends
                    | TypeCompletionContext::ReturnType
                    | TypeCompletionContext::ParameterType
                    | TypeCompletionContext::PropertyType
            ) {
                items.extend(
                    index
                        .types_matching(prefix)
                        .into_iter()
                        .filter(|item| {
                            type_site
                                .as_ref()
                                .map(|site| vendor_type_kind_for_site(site, item.kind))
                                .unwrap_or_else(|| {
                                    vendor_type_kind_allowed(type_context, item.kind)
                                })
                        })
                        .map(|item| {
                            let label = item.short_name;
                            let import = self.composer_import_edit(
                                text,
                                &item.fqn,
                                self.document.cursor_offset(),
                            );
                            CompletionItem {
                                label,
                                detail: Some(format!("{} - Vendor", item.fqn)),
                                kind: Some(vendor_completion_kind(item.kind)),
                                additional_text_edits: import.map(|edit| vec![edit]),
                                ..Default::default()
                            }
                        }),
                );
            } else {
                items.extend(index.classes_matching(prefix).into_iter().map(|fqn| {
                    let label = fqn.rsplit('\\').next().unwrap_or(&fqn).to_owned();
                    let import =
                        self.composer_import_edit(text, &fqn, self.document.cursor_offset());
                    CompletionItem {
                        label,
                        detail: Some(format!("{fqn} • Vendor")),
                        kind: Some(CompletionItemKind::CLASS),
                        additional_text_edits: import.map(|edit| vec![edit]),
                        ..Default::default()
                    }
                }));
            }
        }
        if preceded_by_new {
            rank_new_completion_items(&mut items, prefix);
        }
        let mut seen = std::collections::HashSet::new();
        items.retain(|item| {
            seen.insert(format!(
                "{:?}:{}",
                item.kind,
                item.label.to_ascii_lowercase(),
            ))
        });
        NativeCompletionBatch {
            items: items.into_iter().take(40).collect(),
            new_prefix: (preceded_by_new || pipe_rhs_context).then_some(start..cursor),
        }
    }

    pub(super) fn phpdoc_completion_tags(&self, text: &str, cursor: usize) -> Option<&'static [&'static str]> {
        let target = self.phpdoc_completion_target(text, cursor)?;
        Some(match target {
            DocblockTarget::Callable { .. } => &PHPDOC_CALLABLE_TAGS,
            DocblockTarget::Property { .. } => &PHPDOC_PROPERTY_TAGS,
            DocblockTarget::Type { .. } => &PHPDOC_TYPE_TAGS,
        })
    }

    pub(super) fn phpdoc_completion_target(&self, text: &str, cursor: usize) -> Option<DocblockTarget> {
        let opening = text[..cursor].rfind("/**")?;
        let closing = text[cursor..].find("*/")? + cursor + 2;
        let declaration_start = text[closing..]
            .char_indices()
            .find(|(_, character)| !character.is_whitespace())
            .map(|(offset, _)| closing + offset)?;
        let declaration_end = text[declaration_start..]
            .find('\n')
            .map_or(text.len(), |offset| declaration_start + offset);
        let declaration = &text[declaration_start..declaration_end];
        self.syntax
            .as_ref()
            .and_then(|syntax| docblock_target_at(syntax, declaration_start, declaration))
            .or_else(|| {
                let _ = opening;
                docblock_target_from_source(
                    declaration,
                    &declaration[..declaration.len() - declaration.trim_start().len()],
                )
            })
    }

    #[allow(dead_code)]
    pub fn resident_phpdoc_callable_symbol_id(&self, cursor: usize) -> Option<SymbolId> {
        let syntax = self.syntax.as_ref()?;
        if !self.syntax_is_current_for(self.document.buffer_revision()) {
            return None;
        }
        let docblock_range = docblock_range_at(syntax.text(), cursor)?;
        callable_node_for_docblock(syntax, &docblock_range)?;
        let fingerprint = self.current_syntax_fingerprint()?;
        let snapshot = self.semantic_engine.as_ref()?.try_snapshot()?;
        let file_key = PersistentFileKey::workspace_lexical(&self.file_path);
        let file = snapshot.file_id(&file_key)?;
        if !snapshot.matches_file_fingerprint(file, fingerprint) {
            return None;
        }
        let symbol_id = snapshot.callable_for_docblock(file, &docblock_range)?;
        matches!(
            snapshot.symbol(symbol_id)?.kind,
            ProjectSymbolKind::Function | ProjectSymbolKind::Method
        )
        .then_some(symbol_id)
    }

    pub(super) fn explicit_throws_for_docblock(&self, text: &str, cursor: usize) -> Vec<String> {
        let Some(syntax) = &self.syntax else {
            return Vec::new();
        };
        let Some(close_offset) = text[cursor..].find("*/") else {
            return Vec::new();
        };
        let closing = cursor + close_offset + 2;
        let Some(declaration_start) = text[closing..]
            .char_indices()
            .find(|(_, character)| !character.is_whitespace())
            .map(|(offset, _)| closing + offset)
        else {
            return Vec::new();
        };
        let Some(mut node) = syntax
            .tree()
            .root_node()
            .descendant_for_byte_range(declaration_start, declaration_start + 1)
        else {
            return Vec::new();
        };
        let mut callable = None;
        loop {
            let current = node;
            if matches!(current.kind(), "function_definition" | "method_declaration") {
                callable = Some(current);
                break;
            }
            let Some(parent) = current.parent() else {
                break;
            };
            node = parent;
        }
        let Some(body) = callable.and_then(|node| node.child_by_field_name("body")) else {
            return Vec::new();
        };
        let mut stack = vec![body];
        let mut exceptions = Vec::new();
        while let Some(node) = stack.pop() {
            if matches!(node.kind(), "anonymous_function" | "arrow_function") {
                continue;
            }
            if node.kind() == "throw_expression"
                && let Some(expression) = node.named_children(&mut node.walk()).next()
                && expression.kind() == "object_creation_expression"
                && let Some(class) = expression.child_by_field_name("class").or_else(|| {
                    expression
                        .named_children(&mut expression.walk())
                        .find(|child| matches!(child.kind(), "name" | "qualified_name"))
                })
            {
                let written = syntax_node_text(class, syntax.text()).unwrap_or_default();
                if !written.trim().is_empty() {
                    let written = written.trim();
                    let exception = if written.starts_with('\\') {
                        written.trim_start_matches('\\').to_owned()
                    } else {
                        resolve_php_class_name_at(written, text, class.start_byte())
                    };
                    if !exceptions.contains(&exception) {
                        exceptions.push(exception);
                    }
                }
            }
            stack.extend(node.named_children(&mut node.walk()));
        }
        exceptions
    }

    pub(super) fn local_variable_completions(&self, context: &str, prefix: &str) -> Vec<CompletionItem> {
        let Some(snapshot) = self
            .semantic_engine
            .as_ref()
            .and_then(|engine| engine.try_snapshot())
        else {
            return Vec::new();
        };
        let key = PersistentFileKey::workspace_lexical(&self.file_path);
        let Some(scope) = snapshot.completion_scope(&key, context.len().saturating_sub(1)) else {
            return Vec::new();
        };
        let mut names = std::collections::HashSet::new();
        let mut items: Vec<CompletionItem> = Vec::new();
        for binding in scope
            .bindings
            .iter()
            .filter(|binding| {
                binding.declaration_span.end <= context.len().saturating_sub(prefix.len())
            })
            .filter(|binding| {
                binding.name.starts_with(prefix) && names.insert(binding.name.as_str())
            })
        {
            let position = items.partition_point(|item| {
                (item.label.len(), item.label.as_str())
                    < (binding.name.len(), binding.name.as_str())
            });
            if position >= 40 {
                continue;
            }
            items.insert(
                position,
                CompletionItem {
                    label: binding.name.clone(),
                    kind: Some(CompletionItemKind::VARIABLE),
                    ..Default::default()
                },
            );
            items.truncate(40);
        }
        items
    }

    /// Builds a single additional edit for a Composer/project class. The edit
    /// is deliberately narrow: it only inserts a missing `use` statement and
    /// never rewrites or reformats the document.
    pub(super) fn composer_import_edit(
        &self,
        text: &str,
        fqn: &str,
        caret: usize,
    ) -> Option<lsp_types::TextEdit> {
        let fqn = fqn.trim_start_matches('\\');
        let caret = caret.min(text.len());
        let mut ns_start = 0usize;
        let mut current_namespace = String::new();
        let mut current_bracketed = false;
        for (offset, line) in text.split_inclusive('\n').scan(0usize, |o, line| {
            let s = *o;
            *o += line.len();
            Some((s, line))
        }) {
            if offset > caret {
                break;
            }
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("namespace ") {
                let rest = rest.trim();
                current_namespace = rest
                    .trim_end_matches([';', '{'])
                    .trim()
                    .trim_matches('\\')
                    .to_owned();
                current_bracketed = rest.contains('{');
                ns_start = offset + line.len();
            }
        }
        let target_ns = fqn.rsplit_once('\\').map(|(ns, _)| ns).unwrap_or("");
        if current_namespace.is_empty()
            || current_namespace == target_ns
            || has_import_in_region(&text, fqn, ns_start, caret)
        {
            return None;
        }
        let (region_start, region_end) = if current_bracketed {
            let mut depth = 0i32;
            let mut end = text.len();
            for (i, ch) in text[ns_start..].char_indices() {
                if ch == '{' {
                    depth += 1;
                } else if ch == '}' {
                    if depth == 0 {
                        end = ns_start + i;
                        break;
                    }
                    depth -= 1;
                }
            }
            (ns_start, end.min(caret.max(ns_start)))
        } else {
            (0, text.len())
        };
        let region = &text[region_start..region_end];
        let insertion = region
            .split_inclusive('\n')
            .scan(region_start, |o, line| {
                let s = *o;
                *o += line.len();
                Some((s, line))
            })
            .filter(|(_, line)| line.trim().starts_with("use ") && line.trim_end().ends_with(';'))
            .map(|(o, line)| o + line.len())
            .last()
            .unwrap_or(ns_start);
        let prefix = if insertion > 0 && !text[..insertion].ends_with('\n') {
            "\n"
        } else {
            ""
        };
        let position = PositionCodec::offset_to_position(
            &text,
            insertion,
            self.lsp.as_ref().map(|l| l.encoding()).unwrap_or_default(),
        );
        Some(lsp_types::TextEdit {
            range: lsp_types::Range::new(position, position),
            new_text: format!("{prefix}use {fqn};\n"),
        })
    }
    pub(super) fn resolve_native_type(&self, owner: &str, context: &str) -> Option<String> {
        let owner = owner.trim();
        if let Some(instantiation) = owner.strip_prefix("new ") {
            let class_name = instantiation
                .trim_start()
                .split(['(', ' ', '\t', '\r', '\n'])
                .next()
                .unwrap_or_default();
            if !class_name.is_empty() {
                return Some(self.qualify_type(class_name));
            }
        }
        if owner == "$this" {
            let resolved = declared_class_fqn(context);
            if std::env::var_os("AXIOM_DEBUG_DEFINITION").is_some() {
                eprintln!(
                    "[DEFINITION RECEIVER TYPE] owner={owner:?} candidate={:?} final={:?}",
                    "this", resolved
                );
            }
            return resolved;
        }
        let candidate = owner.trim_start_matches('$').to_owned();
        if std::env::var_os("AXIOM_DEBUG_DEFINITION").is_some() {
            eprintln!("[DEFINITION RECEIVER TYPE] owner={owner:?} candidate={candidate:?}");
        }
        let variable = format!("${candidate}");
        let patterns = [
            format!("{variable} = new "),
            format!("{variable}=new "),
            format!("{variable}: "),
            format!("{variable} : "),
        ];
        for pattern in patterns {
            if let Some(pos) = context.rfind(&pattern) {
                let tail = &context[pos + pattern.len()..];
                let name: String = tail
                    .chars()
                    .take_while(|ch| ch.is_alphanumeric() || *ch == '_' || *ch == '\\')
                    .collect();
                if !name.is_empty() {
                    let resolved = self.qualify_type(&name);
                    return Some(resolved);
                }
            }
        }
        let variable_name = format!("${candidate}");
        let occurrences: Vec<_> = context.match_indices(&variable_name).collect();
        for (pos, _) in occurrences.into_iter().rev() {
            let suffix = &context[pos + variable_name.len()..];
            if std::env::var_os("AXIOM_DEBUG_DEFINITION").is_some() {
                eprintln!(
                    "[DEFINITION RECEIVER OCCURRENCE] owner={owner:?} pos={pos} suffix={suffix:?}"
                );
            }
            // A usage in `$future->await` is not a declaration. The old
            // rfind-based heuristic selected this occurrence and read the
            // preceding statement word (`return`) as its type.
            if suffix.trim_start().starts_with("->") || suffix.trim_start().starts_with("::") {
                continue;
            }
            let declaration = &context[..pos];
            let ty = declaration
                .rsplit(|ch: char| !(ch.is_alphanumeric() || matches!(ch, '_' | '\\' | '?' | '|')))
                .find(|part| !part.is_empty())
                .unwrap_or_default()
                .trim_start_matches('?')
                .trim_matches('|');
            if std::env::var_os("AXIOM_DEBUG_DEFINITION").is_some() {
                eprintln!("[DEFINITION RECEIVER TYPE] declaration={declaration:?} ty={ty:?}");
            }
            if !ty.is_empty()
                && ty
                    .chars()
                    .all(|ch| ch.is_alphanumeric() || ch == '_' || ch == '\\')
            {
                return Some(self.qualify_type(ty));
            }
        }
        let lower = candidate.to_ascii_lowercase();
        if self.runtime_symbols.as_ref().is_some_and(|index| {
            index.find_class(&candidate).is_some()
                || index.find_class(&format!("\\{candidate}")).is_some()
        }) {
            return Some(candidate);
        }
        let resolved = self
            .project_symbols
            .as_ref()
            .and_then(|index| index.try_read().ok())
            .and_then(|index| {
                index
                    .find_class(&candidate)
                    .map(|symbol| symbol.fully_qualified_name.clone())
            })
            .or_else(|| (!lower.is_empty()).then_some(candidate));
        if std::env::var_os("AXIOM_DEBUG_DEFINITION").is_some() {
            eprintln!("[DEFINITION RECEIVER TYPE] owner={owner:?} final={resolved:?}");
        }
        resolved
    }

    /// Resolves a receiver expression, including the common two-level
    /// `$this->property->method()` form. Property declarations are read from
    /// the current document so completion/definition can use their declared
    /// type without guessing from a variable name.
    pub(super) fn resolve_receiver_type(&self, owner: &str, context: &str) -> Option<String> {
        let owner = owner.trim();
        if let Some((base, property)) = owner.rsplit_once("->") {
            let _ = if base.trim() == "$this" {
                declared_class_fqn(context)
            } else {
                self.resolve_receiver_type(base, context)
                    .or_else(|| self.resolve_native_type(base, context))
            }?;
            let property_type = property_type_in_context(context, property.trim())?;
            let resolved = self.resolve_class_name(&property_type, context);
            return Some(resolved);
        }
        self.resolve_native_type(owner, context)
    }

    pub(super) fn qualify_type(&self, name: &str) -> String {
        self.resolve_class_name(name, &self.document.content())
    }

    pub(super) fn resolve_class_name(&self, written: &str, context: &str) -> String {
        let candidate = resolve_php_class_name(written, context);
        let written = written.trim().trim_start_matches('\\');
        if written.contains('\\') {
            return candidate;
        }
        let contextual_exists = self
            .runtime_symbols
            .as_ref()
            .is_some_and(|index| index.find_class(&candidate).is_some())
            || self
                .project_symbols
                .as_ref()
                .and_then(|index| index.try_read().ok())
                .is_some_and(|index| index.find_class(&candidate).is_some());
        // Workspace symbols are authoritative. This keeps project-file
        // activation off the Composer path, which otherwise performs repeated
        // filesystem probes for every imported project type.
        if contextual_exists {
            return candidate;
        }
        let global_exists = self
            .runtime_symbols
            .as_ref()
            .is_some_and(|index| index.find_class(written).is_some())
            || self
                .project_symbols
                .as_ref()
                .and_then(|index| index.try_read().ok())
                .is_some_and(|index| index.find_class(written).is_some());
        if global_exists {
            return written.to_owned();
        }
        self.runtime_symbols
            .as_ref()
            .and_then(|runtime| runtime.find_class_by_short_name(written))
            .map(|symbol| symbol.fqn.clone())
            .unwrap_or(candidate)
    }


    pub(super) fn accept_completion(&mut self, cx: &mut Context<Self>) {
        let Some(item) = self.completions.get(self.completion_selected).cloned() else {
            return;
        };
        let is_asymmetric_visibility = item
            .insert_text
            .as_deref()
            .unwrap_or(&item.label)
            .contains("(set)");
        let is_type_item = is_php_type_completion(
            &item,
            item.insert_text.as_deref().unwrap_or(&item.label),
        );
        let content = self.document.content();
        let encoding = self
            .lsp
            .as_ref()
            .map(|lsp| lsp.encoding())
            .unwrap_or_default();
        let (range, mut text) = match item.text_edit {
            Some(CompletionTextEdit::Edit(edit)) => (
                Self::lsp_range_to_bytes_in(&content, edit.range, encoding),
                edit.new_text,
            ),
            Some(CompletionTextEdit::InsertAndReplace(edit)) => (
                Self::lsp_range_to_bytes_in(&content, edit.replace, encoding),
                edit.new_text,
            ),
            None => (
                self.completion_replacement_range(&content),
                item.insert_text.unwrap_or_else(|| item.label.clone()),
            ),
        };
        let snippet_expansion = (item.insert_text_format == Some(InsertTextFormat::SNIPPET))
            .then(|| snippet::expand(&text));
        let can_start_snippet_session = snippet_expansion.is_some()
            && item
                .additional_text_edits
                .as_ref()
                .is_none_or(|edits| edits.is_empty());
        if let Some(expansion) = &snippet_expansion {
            text = expansion.text.clone();
        }
        let auto_space_type_completion = item.insert_text_format != Some(InsertTextFormat::SNIPPET)
            && is_type_item
            && (parameter_type_context(&content, range.start, range.end)
                || class_member_modifier_type_context(&content, range.start, range.end))
            || (item.insert_text_format != Some(InsertTextFormat::SNIPPET)
                && is_asymmetric_visibility
                && is_inside_class_body(&content[..range.start]));
        let caret_after_existing_space = auto_space_type_completion
            && content[range.end..].starts_with(' ');
        if auto_space_type_completion && !caret_after_existing_space {
            text.push(' ');
        }
        let call_like = matches!(
            item.kind,
            Some(CompletionItemKind::FUNCTION)
                | Some(CompletionItemKind::METHOD)
                | Some(CompletionItemKind::CONSTRUCTOR)
        ) || (item.kind == Some(CompletionItemKind::CLASS)
            && content[..range.start].trim_end().ends_with("new"));
        let has_open = content[range.end..].starts_with('(')
            || text.ends_with('(')
            || text.ends_with(')')
            || text.contains("()");
        if call_like && !has_open {
            text.push('(');
            text.push(')');
        }
        let mut edits = item.additional_text_edits.unwrap_or_default();
        let (_, mapped_start) = axiom_lsp::apply_text_edits_with_offsets(
            &content,
            &edits,
            encoding,
            &[range.start],
        );
        let inserted_len = text.len();
        let main_range = lsp_types::Range::new(
            PositionCodec::offset_to_position(&content, range.start, encoding),
            PositionCodec::offset_to_position(&content, range.end, encoding),
        );
        edits.push(lsp_types::TextEdit {
            range: main_range,
            new_text: text,
        });
        let updated = axiom_lsp::apply_text_edits(&content, &edits, encoding);
        self.suppress_completion_refresh_once = true;
        let mut caret = mapped_start[0]
            .saturating_add(inserted_len)
            .min(updated.len());
        if caret_after_existing_space {
            caret = caret.saturating_add(1).min(updated.len());
        }
        self.document.select_all();
        self.document.insert_text(&updated);
        if can_start_snippet_session {
            let expansion = snippet_expansion.expect("snippet expansion exists");
            let base = range.start;
            let placeholders = expansion
                .placeholders
                .into_iter()
                .map(|mut placeholder| {
                    placeholder.range.start += base;
                    placeholder.range.end += base;
                    placeholder
                })
                .collect();
            self.snippet_session = SnippetSession::new(placeholders);
            if let Some(session) = &self.snippet_session {
                if let Some(active) = session.active_range() {
                    self.document.set_selection(active.start, active.end);
                } else {
                    self.document.set_selection(caret, caret);
                }
            }
        } else {
            self.snippet_session = None;
            self.document.set_selection(caret, caret);
        }
        self.completions.clear();
        self.after_edit(cx);
    }


    pub fn set_completions(&mut self, items: Vec<CompletionItem>, cx: &mut Context<Self>) {
        let text = self.document.content();
        let cursor = self.document.cursor_offset().min(text.len());
        let token_start = php_type_token_range(&text, cursor).start;
        let modifier_type_context =
            class_member_modifier_type_context(&text, token_start, cursor);
        let items = if modifier_type_context {
            items
                .into_iter()
                .filter(|item| {
                    matches!(
                        item.kind,
                        Some(
                            CompletionItemKind::KEYWORD
                                | CompletionItemKind::SNIPPET
                                | CompletionItemKind::CLASS
                                | CompletionItemKind::INTERFACE
                                | CompletionItemKind::ENUM
                                | CompletionItemKind::STRUCT
                        )
                    )
                })
                .collect()
        } else {
            items
        };
        let mut merged = self.completions.clone();
        merged.extend(items);
        let mut seen = std::collections::HashSet::new();
        merged.retain(|item| seen.insert(item.label.to_ascii_lowercase()));
        self.completions = merged;
        self.completion_selected = 0;
        self.completion_scroll.scroll_to_item(0);
        cx.notify();
    }

}

#[cfg(test)]
mod completion_expression_tests {
    use super::extract_owner_expression;

    #[test]
    fn extracts_new_and_chained_call_receivers_before_member_access() {
        for source in [
            "new PhpVersion()->",
            "new PhpVersion()->getVersion()->",
            "new PhpVersion->",
        ] {
            let end = source.rfind("->").unwrap();
            let (start, owner) = extract_owner_expression(source, end);
            assert_eq!(&source[start..end], owner);
            assert!(owner.starts_with("new PhpVersion"));
        }
    }
}

fn php_property_hook_completions(
    text: &str,
    cursor: usize,
) -> Option<(Range<usize>, Vec<CompletionItem>)> {
    let before = text.get(..cursor)?;
    let (open, _declaration, property_name) = before
        .match_indices('{')
        .rev()
        .find_map(|(open, _)| {
            let declaration = before[..open].trim_end();
            let property_name = declaration.rsplit('$').next()?;
            let valid_property = !property_name.is_empty()
                && property_name
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
                && !declaration[declaration.rfind([';', '{', '}']).map_or(0, |i| i + 1)..]
                    .contains("function");
            valid_property.then_some((open, declaration, property_name))
        })?;
    let mut depth = 1usize;
    for ch in before[open + 1..].chars() {
        match ch {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    if depth == 0 {
        return None;
    }

    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    let line = &before[line_start..];
    let variable_start = line
        .char_indices()
        .rev()
        .take_while(|(_, ch)| ch.is_ascii_alphanumeric() || *ch == '_' || *ch == '$')
        .last()
        .map_or(cursor, |(i, _)| line_start + i);
    let token = &text[variable_start..cursor];
    let hook_body = &before[open + 1..];
    let latest_get = hook_body.rfind("get");
    let latest_set = hook_body.rfind("set");
    let in_set_hook = latest_set.is_some_and(|set| latest_get.is_none_or(|get| set > get));
    if token.starts_with('$') {
        let items = (in_set_hook && "$value".starts_with(token))
            .then(|| CompletionItem {
                label: "$value".into(),
                detail: Some("PHP 8.4 implicit property-hook value".into()),
                kind: Some(CompletionItemKind::VARIABLE),
                insert_text: Some("$value".into()),
                ..Default::default()
            })
            .into_iter()
            .collect();
        return Some((variable_start..cursor, items));
    }
    let start = token
        .char_indices()
        .rev()
        .take_while(|(_, ch)| ch.is_ascii_alphabetic())
        .last()
        .map_or(cursor, |(i, _)| variable_start + i);
    let prefix = &text[start..cursor];
    let latest_hook = latest_get.into_iter().chain(latest_set).max();
    let cursor_in_hook_body = latest_hook.is_some_and(|hook| {
        let suffix = &hook_body[hook + 3..];
        suffix.contains("=>") || suffix.contains('{') || suffix.contains('(')
    });
    if cursor_in_hook_body || prefix.is_empty() {
        return None;
    }
    let indent = before[..line_start]
        .rsplit('\n')
        .next()
        .unwrap_or_default()
        .chars()
        .take_while(|ch| ch.is_whitespace())
        .collect::<String>();
    let nested_indent = format!("{indent}    ");
    let snippets = [
        ("get =>", format!("get => ${{1:$this->_{property_name}}}")),
        ("set =>", "set => ${1:strtolower($value)}".into()),
        (
            "get { ... }",
            format!("get {{\n{nested_indent}return ${{1}};\n{indent}}}"),
        ),
        (
            "set { ... }",
            format!("set {{\n{nested_indent}$this->${{1}} = $value;\n{indent}}}"),
        ),
    ];
    let mut items = vec!["get", "set"]
        .into_iter()
        .filter(|label| label.starts_with(prefix))
        .map(|label| CompletionItem {
            label: label.into(),
            detail: Some("PHP 8.4 property hook".into()),
            kind: Some(CompletionItemKind::KEYWORD),
            insert_text: Some(label.into()),
            ..Default::default()
        })
        .collect::<Vec<_>>();
    items.extend(
        snippets
            .into_iter()
            .filter(|(label, _)| label.starts_with(prefix))
            .map(|(label, insert_text)| CompletionItem {
                label: label.into(),
                detail: Some("PHP 8.4 property hook snippet".into()),
                kind: Some(CompletionItemKind::SNIPPET),
                insert_text: Some(insert_text),
                insert_text_format: Some(InsertTextFormat::SNIPPET),
                ..Default::default()
            }),
    );
    (!items.is_empty()).then_some((start..cursor, items))
}

#[cfg(test)]
mod property_hook_completion_tests {
    use super::php_property_hook_completions;

    #[test]
    fn offers_hook_keywords_snippets_and_set_value() {
        let source = "<?php class C { public string $name { ge";
        let (_, items) = php_property_hook_completions(source, source.len()).unwrap();
        assert!(items.iter().any(|item| item.label == "get"));
        assert!(items.iter().any(|item| item.label == "get =>"));

        let source = "<?php class C { public string $name { set { $v";
        let (_, items) = php_property_hook_completions(source, source.len()).unwrap();
        assert!(items.iter().any(|item| item.label == "$value"));
    }
}

pub(super) fn php_type_completion_site(text: &str, cursor: usize) -> Option<PhpTypeCompletionSite> {
    let cursor = cursor.min(text.len());
    if !text.is_char_boundary(cursor) {
        return None;
    }
    let range = php_type_token_range(text, cursor);
    // Incomplete class-member declarations are commonly rejected by the
    // parser. Recognize their modifier + type prefix lexically first so they
    // still use type-only candidates (and never fall through to runtime funcs).
    if class_member_modifier_type_context(text, range.start, cursor) {
        return Some(PhpTypeCompletionSite {
            range,
            context: TypeCompletionContext::PropertyType,
            nullable: false,
            connector: None,
            class_scope: true,
        });
    }
    if php_lexical_state(&text[..cursor]) != PhpLexicalState::Code {
        return None;
    }
    let before = text[..range.start].trim_end();
    let (nullable, before) = if before.ends_with('?') {
        (true, before[..before.len() - '?'.len_utf8()].trim_end())
    } else {
        (false, before)
    };
    let (connector, _) = match before.chars().next_back() {
        Some('|') => (
            Some(TypeConnector::Union),
            before[..before.len() - '|'.len_utf8()].trim_end(),
        ),
        Some('&') => (
            Some(TypeConnector::Intersection),
            before[..before.len() - '&'.len_utf8()].trim_end(),
        ),
        _ => (None, before),
    };
    let context = if return_type_context(text, range.start, cursor) {
        TypeCompletionContext::ReturnType
    } else if parameter_type_context(text, range.start, cursor) {
        TypeCompletionContext::ParameterType
    } else if property_type_context(text, cursor) {
        TypeCompletionContext::PropertyType
    } else {
        return None;
    };
    let class_scope = is_inside_class_body(&text[..range.start]);
    Some(PhpTypeCompletionSite {
        range,
        context,
        nullable,
        connector,
        class_scope,
    })
}

pub(super) fn php_type_token_range(text: &str, cursor: usize) -> Range<usize> {
    let start = text[..cursor]
        .char_indices()
        .rev()
        .take_while(|(_, character)| character.is_alphanumeric() || matches!(character, '_' | '\\'))
        .last()
        .map_or(cursor, |(offset, _)| offset);
    start..cursor
}

pub(super) fn return_type_context(text: &str, component_start: usize, component_end: usize) -> bool {
    let before = &text[..component_start];
    let Some(colon) = before.char_indices().rev().find_map(|(offset, character)| {
        (character == ':' && !is_double_colon(before, offset)).then_some(offset)
    }) else {
        return false;
    };
    let declaration = before[..colon].trim_end();
    if !declaration.ends_with(')') {
        return false;
    }
    let close = declaration.len() - ')'.len_utf8();
    let Some(open) = matching_open_paren(declaration, close) else {
        return false;
    };
    callable_keyword_before_paren(declaration, open)
        && is_partial_type_expression(&text[colon + ':'.len_utf8()..component_end])
}

pub(super) fn parameter_type_context(text: &str, component_start: usize, component_end: usize) -> bool {
    let before = &text[..component_start];
    let end = before.len();
    let Some(open) = enclosing_open_paren(before, end) else {
        return false;
    };
    if !callable_keyword_before_paren(before, open) {
        return false;
    }
    let Some(parameter_start) = parameter_type_start(before, open + 1, end) else {
        return false;
    };
    is_partial_type_expression(&text[parameter_start..component_end])
}

pub(super) fn is_php_type_completion(item: &CompletionItem, inserted_text: &str) -> bool {
    if item.kind == Some(CompletionItemKind::CLASS) {
        return true;
    }
    let name = inserted_text.trim().trim_start_matches('\\');
    matches!(
        name.to_ascii_lowercase().as_str(),
        "int" | "string" | "bool" | "float" | "array" | "object" | "mixed"
    )
}

pub(super) fn property_type_context(text: &str, cursor: usize) -> bool {
    let before = &text[..cursor];
    let line_start = before.rfind('\n').map_or(0, |offset| offset + 1);
    let line_before = &before[line_start..];
    if !is_inside_class_body(before) {
        return false;
    }
    let line_after = text[cursor..]
        .split(['\n', '\r'])
        .next()
        .unwrap_or_default();
    let Some(type_prefix) = property_type_prefix(line_before) else {
        return false;
    };
    let Some(variable) = line_after.find('$') else {
        return line_after.trim().is_empty() && is_partial_type_expression(type_prefix);
    };
    let type_expression = format!("{type_prefix}{}", &line_after[..variable]);
    let name = line_after[variable + '$'.len_utf8()..]
        .chars()
        .next()
        .is_some_and(|character| character == '_' || character.is_alphabetic());
    name && is_partial_type_expression(&type_expression)
}

pub(super) fn class_member_modifier_type_context(text: &str, token_start: usize, cursor: usize) -> bool {
    if token_start > cursor
        || !text.is_char_boundary(token_start)
        || !text.is_char_boundary(cursor)
    {
        return false;
    }
    // Keep this deliberately lexical: incomplete declarations (and earlier
    // syntax errors elsewhere in the file) must not disable type completion.
    let line_start = text[..token_start]
        .rfind('\n')
        .map_or(0, |offset| offset + 1);
    let modifiers = text[line_start..token_start].split_whitespace().collect::<Vec<_>>();
    if modifiers.is_empty()
        || modifiers.iter().any(|modifier| {
            !matches!(
                *modifier,
                "public"
                    | "protected"
                    | "private"
                    | "public(set)"
                    | "protected(set)"
                    | "private(set)"
                    | "readonly"
                    | "static"
                    | "final"
                    | "abstract"
            )
        })
    {
        return false;
    }
    is_partial_type_expression(&text[token_start..cursor])
}

pub(super) fn property_type_prefix(line: &str) -> Option<&str> {
    let mut remaining = line.trim_start();
    let mut modifier_count = 0;
    loop {
        let word_end = remaining
            .find(|character: char| character.is_whitespace())
            .unwrap_or(remaining.len());
        if !matches!(
            &remaining[..word_end],
            "public"
                | "protected"
                | "private"
                | "public(set)"
                | "protected(set)"
                | "private(set)"
                | "var"
                | "static"
                | "readonly"
        ) {
            break;
        }
        modifier_count += 1;
        remaining = remaining[word_end..].trim_start();
    }
    (modifier_count > 0 && !remaining.contains(['=', '(', ')', '{', '}', ';'])).then_some(remaining)
}

pub(super) fn parameter_type_start(text: &str, start: usize, end: usize) -> Option<usize> {
    let mut paren_depth = 0usize;
    let mut bracket_depth = 0usize;
    let mut brace_depth = 0usize;
    let mut type_start = start;
    for (offset, character) in text[start..end].char_indices() {
        let absolute = start + offset;
        match character {
            '(' => paren_depth += 1,
            ')' if paren_depth > 0 => paren_depth -= 1,
            '[' => bracket_depth += 1,
            ']' if bracket_depth > 0 => bracket_depth -= 1,
            '{' => brace_depth += 1,
            '}' if brace_depth > 0 => brace_depth -= 1,
            '=' if paren_depth == 0 && bracket_depth == 0 && brace_depth == 0 => return None,
            ',' if paren_depth == 0 && bracket_depth == 0 && brace_depth == 0 => {
                type_start = absolute + character.len_utf8();
            }
            _ => {}
        }
    }
    Some(type_start)
}

pub(super) fn is_partial_type_expression(value: &str) -> bool {
    let value = value.trim();
    if value.is_empty() {
        return true;
    }
    let mut component_start = 0;
    let mut first = true;
    let mut connector = None;
    for (offset, character) in value.char_indices() {
        if !matches!(character, '|' | '&') {
            continue;
        }
        let component = value[component_start..offset].trim();
        if !is_partial_type_component(component, first) {
            return false;
        }
        if first && component.starts_with('?') {
            return false;
        }
        let current_connector = if character == '|' {
            TypeConnector::Union
        } else {
            TypeConnector::Intersection
        };
        if connector.is_some_and(|previous| previous != current_connector) {
            return false;
        }
        connector = Some(current_connector);
        component_start = offset + character.len_utf8();
        first = false;
    }
    let component = value[component_start..].trim();
    component.is_empty() || is_partial_type_component(component, first)
}

pub(super) fn is_partial_type_component(value: &str, first: bool) -> bool {
    let value = value.trim();
    if value.is_empty() {
        return false;
    }
    let nullable = first && value.starts_with('?');
    let value = if nullable {
        &value['?'.len_utf8()..]
    } else {
        value
    };
    if value.is_empty() {
        return nullable;
    }
    is_partial_type_name(value)
}

pub(super) fn is_partial_type_name(value: &str) -> bool {
    let value = value.strip_prefix('\\').unwrap_or(value);
    let mut segment_start = 0;
    for (offset, character) in value.char_indices() {
        if character != '\\' {
            continue;
        }
        if !is_partial_type_name_segment(&value[segment_start..offset]) {
            return false;
        }
        segment_start = offset + character.len_utf8();
    }
    value[segment_start..].is_empty() || is_partial_type_name_segment(&value[segment_start..])
}

pub(super) fn is_partial_type_name_segment(value: &str) -> bool {
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    (first == '_' || first.is_alphabetic() || !first.is_ascii())
        && characters.all(|character| {
            character == '_' || character.is_alphanumeric() || !character.is_ascii()
        })
}

pub(super) fn matching_open_paren(text: &str, close: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (offset, character) in text[..=close].char_indices().rev() {
        match character {
            ')' => depth += 1,
            '(' if depth == 1 => return Some(offset),
            '(' => depth -= 1,
            _ => {}
        }
    }
    None
}

pub(super) fn enclosing_open_paren(text: &str, end: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (offset, character) in text[..end].char_indices().rev() {
        match character {
            ')' => depth += 1,
            '(' if depth == 0 => return Some(offset),
            '(' => depth -= 1,
            _ => {}
        }
    }
    None
}

pub(super) fn callable_keyword_before_paren(text: &str, open: usize) -> bool {
    let head = text[..open].trim_end();
    if ends_with_word(head, "function") || ends_with_word(head, "fn") {
        return true;
    }
    let name_start = head
        .char_indices()
        .rev()
        .take_while(|(_, character)| character.is_alphanumeric() || matches!(character, '_' | '\\'))
        .last()
        .map_or(head.len(), |(offset, _)| offset);
    let declaration = head[..name_start].trim_end();
    ends_with_word(declaration, "function") || ends_with_word(declaration, "fn")
}

pub(super) fn ends_with_word(text: &str, word: &str) -> bool {
    let Some(offset) = text.strip_suffix(word).map(|value| value.len()) else {
        return false;
    };
    text[..offset]
        .chars()
        .next_back()
        .is_none_or(|character| !character.is_alphanumeric() && !matches!(character, '_' | '\\'))
}

pub(super) fn is_double_colon(text: &str, offset: usize) -> bool {
    text.as_bytes()
        .get(offset.wrapping_sub(1))
        .is_some_and(|byte| *byte == b':')
        || text
            .as_bytes()
            .get(offset + 1)
            .is_some_and(|byte| *byte == b':')
}

pub(super) fn is_inside_class_body(text: &str) -> bool {
    let Some(open) = last_unclosed_brace(text) else {
        return false;
    };
    let head = text[..open].trim_end();
    if head.ends_with(')') {
        return false;
    }
    ["class", "interface", "trait", "enum"]
        .into_iter()
        .filter_map(|keyword| {
            head.rmatch_indices(keyword).find_map(|(offset, _)| {
                let before_is_boundary = head[..offset]
                    .chars()
                    .next_back()
                    .is_none_or(|character| !character.is_alphanumeric() && character != '_');
                let after = offset + keyword.len();
                let after_is_boundary = head[after..]
                    .chars()
                    .next()
                    .is_none_or(|character| !character.is_alphanumeric() && character != '_');
                (before_is_boundary && after_is_boundary).then_some((offset, keyword))
            })
        })
        .max_by_key(|(offset, _)| *offset)
        .is_some_and(|(offset, keyword)| {
            let declaration_tail = &head[offset + keyword.len()..];
            !declaration_tail.contains('(')
                && !declaration_tail.contains('{')
                && !declaration_tail.contains('}')
                && !declaration_tail.contains(';')
        })
}

pub(super) fn php_class_member_completions(
    text: &str,
    cursor: usize,
) -> Option<(Range<usize>, Vec<CompletionItem>)> {
    let cursor = cursor.min(text.len());
    if !text.is_char_boundary(cursor) || php_lexical_state(&text[..cursor]) != PhpLexicalState::Code
    {
        return None;
    }
    let before = &text[..cursor];
    if !is_inside_class_body(before) {
        return None;
    }
    let line_start = before.rfind('\n').map_or(0, |offset| offset + 1);
    let line = &before[line_start..];
    let prefix_start = line
        .char_indices()
        .rev()
        .take_while(|(_, character)| character.is_ascii_alphabetic())
        .last()
        .map_or(cursor, |(offset, _)| line_start + offset);
    let prefix = &text[prefix_start..cursor];
    let preceding_words = line[..prefix_start - line_start]
        .split_whitespace()
        .collect::<Vec<_>>();
    // An empty prefix after a modifier (for example `protected |`) is still a
    // useful member-completion context: offer the next asymmetric modifier.
    if prefix.is_empty() && preceding_words.is_empty() {
        return None;
    }
    if preceding_words.iter().any(|word| {
        !matches!(
            *word,
            "public"
                | "protected"
                | "private"
                | "readonly"
                | "static"
                | "abstract"
                | "final"
        )
    }) {
        return None;
    }

    let indent = line.chars().take_while(|ch| ch.is_whitespace()).collect::<String>();
    let body_indent = format!("{indent}    ");
    let definitions: [(&str, &[&str], Option<String>); 12] = [
        ("public", &["pub"], None),
        ("protected", &["pro"], None),
        ("private", &["pri"], None),
        ("function", &["fun"], None),
        ("readonly", &["rea"], None),
        ("static", &["sta"], None),
        ("const", &["con"], None),
        ("abstract", &["abs"], None),
        ("final", &["fin"], None),
        (
            "public function",
            &["pub", "pubf", "fun"],
            Some(format!(
                "public function ${{1:methodName}}(${{2}}): ${{3:void}}\n{indent}{{\n{body_indent}$0\n{indent}}}"
            )),
        ),
        (
            "protected function",
            &["pro", "prof", "fun"],
            Some(format!(
                "protected function ${{1:methodName}}(${{2}}): ${{3:void}}\n{indent}{{\n{body_indent}$0\n{indent}}}"
            )),
        ),
        (
            "private function",
            &["pri", "prif", "fun"],
            Some(format!(
                "private function ${{1:methodName}}(${{2}}): ${{3:void}}\n{indent}{{\n{body_indent}$0\n{indent}}}"
            )),
        ),
    ];
    let mut items = definitions
        .into_iter()
        .filter(|(label, aliases, snippet)| {
            let visibility = preceding_words
                .iter()
                .find(|word| matches!(**word, "public" | "protected" | "private"))
                .copied()
                .or_else(|| {
                    match prefix.to_ascii_lowercase().as_str() {
                        "pro" | "prof" => Some("protected"),
                        "pri" | "prif" => Some("private"),
                        "pub" | "pubf" => Some("public"),
                        _ => ["protected", "private", "public"]
                            .into_iter()
                            .find(|visibility| {
                                starts_with_ascii_case_insensitive(visibility, prefix)
                            }),
                    }
                })
                .unwrap_or("public");
            (starts_with_ascii_case_insensitive(label, prefix)
                || aliases
                    .iter()
                    .any(|alias| alias.eq_ignore_ascii_case(prefix)))
                && (snippet.is_none()
                    || !aliases.iter().any(|alias| *alias == "fun")
                    || label.starts_with(visibility))
        })
        .map(|(label, _, snippet)| match snippet {
            Some(insert_text) => CompletionItem {
                label: label.to_owned(),
                detail: Some("PHP class member snippet".to_owned()),
                kind: Some(CompletionItemKind::SNIPPET),
                insert_text: Some(insert_text),
                insert_text_format: Some(InsertTextFormat::SNIPPET),
                ..Default::default()
            },
            None => CompletionItem {
                label: label.to_owned(),
                detail: Some("PHP class member keyword".to_owned()),
                kind: Some(CompletionItemKind::KEYWORD),
                insert_text: Some(label.to_owned()),
                ..Default::default()
            },
        })
        .collect::<Vec<_>>();
    for (visibility, alias) in [("public", "pubc"), ("protected", "proc"), ("private", "pric")] {
        let label = format!("{visibility} const");
        if starts_with_ascii_case_insensitive(&label, prefix)
            || prefix.eq_ignore_ascii_case(alias)
        {
            items.push(CompletionItem {
                label,
                detail: Some("PHP class constant snippet".to_owned()),
                kind: Some(CompletionItemKind::SNIPPET),
                insert_text: Some(format!("{visibility} const ${{1:NAME}} = ${{2:value}};")),
                insert_text_format: Some(InsertTextFormat::SNIPPET),
                ..Default::default()
            });
        }
        for modifier in ["static", "readonly"] {
            let label = format!("{visibility} {modifier}");
            if starts_with_ascii_case_insensitive(&label, prefix) {
                items.push(CompletionItem {
                    label: label.clone(),
                    detail: Some("PHP class member keyword".to_owned()),
                    kind: Some(CompletionItemKind::KEYWORD),
                    insert_text: Some(label),
                    ..Default::default()
                });
            }
        }
    }
    let asymmetric_variants = [
        ("private(set)", "pri"),
        ("protected(set)", "pro"),
        ("public(set)", "pub"),
        ("public private(set)", "pub"),
        ("public protected(set)", "pub"),
        ("protected private(set)", "pro"),
    ];
    for (label, alias) in asymmetric_variants {
        let visible_prefix = !prefix.is_empty()
            && (starts_with_ascii_case_insensitive(label, prefix)
                || prefix.eq_ignore_ascii_case(alias));
        if visible_prefix
            && (preceding_words.is_empty() || !label.contains(' '))
        {
            items.push(CompletionItem {
                label: label.to_owned(),
                detail: Some("PHP 8.4 asymmetric property visibility".to_owned()),
                kind: Some(CompletionItemKind::KEYWORD),
                insert_text: Some(label.to_owned()),
                ..Default::default()
            });
        }
    }
    // Offer only a legal write-visibility continuation after the read
    // visibility already present on the line.
    let read_visibility = preceding_words.last().copied();
    let write_visibility = match read_visibility {
        Some("public") => [
            Some(("public private(set)", "private(set)")),
            Some(("public protected(set)", "protected(set)")),
            Some(("public(set)", "public(set)")),
        ],
        Some("protected") => [
            Some(("protected private(set)", "private(set)")),
            Some(("protected(set)", "protected(set)")),
            None,
        ],
        Some("private") => [Some(("private(set)", "private(set)")), None, None],
        _ => [None, None, None],
    };
    for (label, insert_text) in write_visibility.into_iter().flatten() {
        if prefix.is_empty() || starts_with_ascii_case_insensitive(insert_text, prefix) {
            items.push(CompletionItem {
                label: label.to_owned(),
                detail: Some("PHP 8.4 asymmetric property write visibility".to_owned()),
                kind: Some(CompletionItemKind::KEYWORD),
                insert_text: Some(insert_text.to_owned()),
                ..Default::default()
            });
        }
    }
    (!items.is_empty()).then_some((prefix_start..cursor, items))
}

pub(super) fn last_unclosed_brace(text: &str) -> Option<usize> {
    let mut stack = Vec::new();
    let mut state = PhpLexicalState::Code;
    let mut offset = 0;
    while offset < text.len() {
        let remaining = &text[offset..];
        let advance = match state {
            PhpLexicalState::Code => {
                if remaining.starts_with("//") {
                    state = PhpLexicalState::LineComment;
                    2
                } else if remaining.starts_with("#[") {
                    2
                } else if remaining.starts_with('#') {
                    state = PhpLexicalState::LineComment;
                    1
                } else if remaining.starts_with("/*") {
                    state = if remaining.starts_with("/**") {
                        PhpLexicalState::DocComment
                    } else {
                        PhpLexicalState::BlockComment
                    };
                    2
                } else if remaining.starts_with('\'') {
                    state = PhpLexicalState::SingleQuoted;
                    1
                } else if remaining.starts_with('"') {
                    state = PhpLexicalState::DoubleQuoted;
                    1
                } else if remaining.starts_with('`') {
                    state = PhpLexicalState::Backtick;
                    1
                } else {
                    let character = remaining.chars().next().unwrap();
                    match character {
                        '{' => stack.push(offset),
                        '}' => {
                            stack.pop();
                        }
                        _ => {}
                    }
                    character.len_utf8()
                }
            }
            PhpLexicalState::LineComment => {
                let character = remaining.chars().next().unwrap();
                if matches!(character, '\n' | '\r') {
                    state = PhpLexicalState::Code;
                }
                character.len_utf8()
            }
            PhpLexicalState::BlockComment | PhpLexicalState::DocComment => {
                if remaining.starts_with("*/") {
                    state = PhpLexicalState::Code;
                    2
                } else {
                    remaining.chars().next().unwrap().len_utf8()
                }
            }
            PhpLexicalState::SingleQuoted
            | PhpLexicalState::DoubleQuoted
            | PhpLexicalState::Backtick => {
                let quote = match state {
                    PhpLexicalState::SingleQuoted => '\'',
                    PhpLexicalState::DoubleQuoted => '"',
                    PhpLexicalState::Backtick => '`',
                    _ => unreachable!(),
                };
                if remaining.starts_with('\\') {
                    1 + remaining
                        .get(1..)
                        .and_then(|tail| tail.chars().next())
                        .map_or(0, char::len_utf8)
                } else if remaining.starts_with(quote) {
                    state = PhpLexicalState::Code;
                    quote.len_utf8()
                } else {
                    remaining.chars().next().unwrap().len_utf8()
                }
            }
        };
        offset += advance;
    }
    stack.last().copied()
}

pub(super) fn php_native_type_allowed(site: &PhpTypeCompletionSite, type_name: &str) -> bool {
    let standalone = !site.nullable && site.connector.is_none();
    match type_name {
        "void" | "never" => site.context == TypeCompletionContext::ReturnType && standalone,
        "mixed" => standalone,
        "callable" => {
            site.connector.is_none()
                && matches!(
                    site.context,
                    TypeCompletionContext::ReturnType | TypeCompletionContext::ParameterType
                )
        }
        "iterable" => site.connector.is_none(),
        "null" | "false" | "true" => site.connector != Some(TypeConnector::Intersection),
        "static" => site.context == TypeCompletionContext::ReturnType && site.connector.is_none(),
        "self" | "parent" => site.class_scope,
        _ => site.connector != Some(TypeConnector::Intersection),
    }
}

pub(super) fn type_completion_context(
    before: &str,
    start: usize,
    preceded_by_new: bool,
) -> TypeCompletionContext {
    let head = before[..start].trim_end();
    if preceded_by_new {
        return TypeCompletionContext::New;
    }
    if head.ends_with("implements") {
        return TypeCompletionContext::ClassImplements;
    }
    if head.ends_with("extends") {
        let class_pos = head.rfind("class ");
        let interface_pos = head.rfind("interface ");
        return if interface_pos > class_pos {
            TypeCompletionContext::InterfaceExtends
        } else {
            TypeCompletionContext::ClassExtends
        };
    }
    TypeCompletionContext::GenericType
}

pub(crate) fn type_kind_allowed(context: TypeCompletionContext, kind: ProjectSymbolKind) -> bool {
    match context {
        TypeCompletionContext::New | TypeCompletionContext::ClassExtends => {
            kind == ProjectSymbolKind::Class
        }
        TypeCompletionContext::ClassImplements | TypeCompletionContext::InterfaceExtends => {
            kind == ProjectSymbolKind::Interface
        }
        TypeCompletionContext::ReturnType
        | TypeCompletionContext::ParameterType
        | TypeCompletionContext::PropertyType => matches!(
            kind,
            ProjectSymbolKind::Class
                | ProjectSymbolKind::Interface
                | ProjectSymbolKind::Trait
                | ProjectSymbolKind::Enum
        ),
        TypeCompletionContext::GenericType => true,
    }
}

pub(crate) fn runtime_type_kind_allowed(context: TypeCompletionContext, kind: RuntimeKind) -> bool {
    match context {
        TypeCompletionContext::New | TypeCompletionContext::ClassExtends => {
            kind == RuntimeKind::Class
        }
        TypeCompletionContext::ClassImplements | TypeCompletionContext::InterfaceExtends => {
            kind == RuntimeKind::Interface
        }
        TypeCompletionContext::ReturnType
        | TypeCompletionContext::ParameterType
        | TypeCompletionContext::PropertyType => matches!(
            kind,
            RuntimeKind::Class
                | RuntimeKind::Interface
                | RuntimeKind::Trait
                | RuntimeKind::Enum
        ),
        TypeCompletionContext::GenericType => true,
    }
}

pub(super) fn runtime_global_kind_allowed(
    before: &str,
    followed_by_static_operator: bool,
    prefix: &str,
    start: usize,
    preceded_by_new: bool,
    has_member_operator: bool,
    kind: RuntimeKind,
) -> bool {
    if preceded_by_new || has_member_operator || prefix.is_empty() {
        return false;
    }
    if followed_by_static_operator {
        return matches!(
            kind,
            RuntimeKind::Class | RuntimeKind::Interface | RuntimeKind::Trait | RuntimeKind::Enum
        );
    }
    let head = before[..start].trim_end();
    if head.ends_with('$')
        || head.ends_with('.')
        || head.ends_with("->")
        || head.ends_with("::")
        || [
            "function",
            "class",
            "interface",
            "trait",
            "enum",
            "extends",
            "implements",
            "use",
            "const",
        ]
        .iter()
        .any(|keyword| head.ends_with(keyword))
    {
        return false;
    }
    if prefix.starts_with("PHP_") {
        return kind == RuntimeKind::GlobalConstant;
    }
    prefix
        .chars()
        .next()
        .is_some_and(|character| character.is_lowercase() || character == '_')
        && kind == RuntimeKind::Function
}

pub(super) fn project_type_kind_for_site(site: &PhpTypeCompletionSite, kind: ProjectSymbolKind) -> bool {
    type_kind_allowed(site.context, kind)
        && (site.connector != Some(TypeConnector::Intersection)
            || matches!(
                kind,
                ProjectSymbolKind::Class | ProjectSymbolKind::Interface
            ))
}

pub(super) fn runtime_type_kind_for_site(site: &PhpTypeCompletionSite, kind: RuntimeKind) -> bool {
    runtime_type_kind_allowed(site.context, kind)
        && (site.connector != Some(TypeConnector::Intersection)
            || matches!(kind, RuntimeKind::Class | RuntimeKind::Interface))
}

pub(super) fn vendor_type_kind_allowed(context: TypeCompletionContext, kind: ProjectSymbolKind) -> bool {
    type_kind_allowed(context, kind)
}

pub(super) fn vendor_type_kind_for_site(site: &PhpTypeCompletionSite, kind: ProjectSymbolKind) -> bool {
    project_type_kind_for_site(site, kind)
}

pub(super) fn vendor_completion_kind(kind: ProjectSymbolKind) -> CompletionItemKind {
    match kind {
        ProjectSymbolKind::Interface => CompletionItemKind::INTERFACE,
        ProjectSymbolKind::Enum => CompletionItemKind::ENUM,
        _ => CompletionItemKind::CLASS,
    }
}

pub(super) struct NativeCompletionBatch {
    pub(super) items: Vec<CompletionItem>,
    pub(super) new_prefix: Option<std::ops::Range<usize>>,
}

#[cfg(test)]
pub(super) fn vendor_lookup_needed(project_found: bool) -> bool {
    !project_found
}

#[cfg(test)]
pub(super) fn resolve_vendor_definition_target(
    index: &Arc<std::sync::RwLock<VendorSymbolIndex>>,
    resolved: &str,
) -> Option<(PathBuf, Range<usize>)> {
    let mut index = index.write().ok()?;
    index
        .symbols_of(resolved)
        .into_iter()
        .find(|symbol| {
            matches!(
                symbol.kind,
                ProjectSymbolKind::Class
                    | ProjectSymbolKind::Interface
                    | ProjectSymbolKind::Trait
                    | ProjectSymbolKind::Enum
            )
        })
        .map(|symbol| (symbol.file, symbol.range))
        .or_else(|| index.resolve_class(resolved).map(|file| (file, 0..0)))
}

pub(super) fn resolve_php_class_name(written: &str, context: &str) -> String {
    let written = written.trim().trim_start_matches('\\');
    if matches!(written, "self" | "static") {
        return declared_class_fqn(context).unwrap_or_else(|| written.to_owned());
    }
    if written == "parent" {
        return declared_parent_fqn(context).unwrap_or_else(|| written.to_owned());
    }
    if written.contains('\\') {
        return written.to_owned();
    }
    for (fqn, alias) in context.lines().flat_map(parse_use_imports) {
        if alias == written {
            return fqn;
        }
    }
    context
        .lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix("namespace ")
                .map(|value| value.trim_end_matches(';').trim())
                .filter(|namespace| !namespace.is_empty())
        })
        .map(|namespace| format!("{namespace}\\{written}"))
        .unwrap_or_else(|| written.to_owned())
}

pub(super) fn resolve_php_class_name_at(written: &str, context: &str, offset: usize) -> String {
    let written = written.trim().trim_start_matches('\\');
    if written.contains('\\') {
        return written.to_owned();
    }
    let prefix = &context[..offset.min(context.len())];
    for (fqn, alias) in prefix.lines().flat_map(parse_use_imports) {
        if alias == written {
            return fqn;
        }
    }
    let namespace = prefix
        .lines()
        .filter_map(|line| line.trim().strip_prefix("namespace "))
        .last()
        .map(|value| value.trim_end_matches([';', '{']).trim())
        .filter(|value| !value.is_empty())
        .unwrap_or("");
    if namespace.is_empty() {
        written.to_owned()
    } else {
        format!("{namespace}\\{written}")
    }
}

pub(super) fn split_method_chain(owner: &str) -> Option<(&str, Vec<String>)> {
    let mut parts = owner.split("->");
    let base = parts.next()?.trim();
    let methods: Vec<String> = parts
        .map(|part| part.trim().trim_end_matches("()"))
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect();
    (!methods.is_empty()).then_some((base, methods))
}

/// Returns `(fully-qualified import, local binding)` pairs for PHP `use`
/// declarations, including grouped imports.
pub(super) fn parse_use_imports(line: &str) -> Vec<(String, String)> {
    let Some(value) = line.trim().strip_prefix("use ") else {
        return Vec::new();
    };
    let value = value.split(';').next().unwrap_or(value).trim();
    if let Some((prefix, members)) = value.split_once('{') {
        let prefix = prefix.trim().trim_end_matches('\\');
        return members
            .trim_end_matches('}')
            .split(',')
            .filter_map(|member| {
                let member = member.trim();
                if member.is_empty() {
                    return None;
                }
                let (name, alias) = member
                    .split_once(" as ")
                    .map(|(name, alias)| (name.trim(), alias.trim()))
                    .unwrap_or((member, member.rsplit('\\').next().unwrap_or(member)));
                let fqn = format!("{prefix}\\{}", name.trim_matches('\\'));
                Some((fqn, alias.to_owned()))
            })
            .collect();
    }
    let (fqn, alias) = value
        .split_once(" as ")
        .map(|(fqn, alias)| (fqn.trim(), alias.trim().to_owned()))
        .unwrap_or_else(|| {
            let fqn = value.trim();
            (fqn, fqn.rsplit('\\').next().unwrap_or(fqn).to_owned())
        });
    vec![(fqn.trim_matches('\\').to_owned(), alias)]
}

#[cfg(test)]
pub(super) fn has_import(context: &str, fqn: &str) -> bool {
    context
        .lines()
        .flat_map(parse_use_imports)
        .any(|(import, _)| import == fqn)
}

pub(super) fn extract_owner_expression(text: &str, owner_end: usize) -> (usize, String) {
    let prefix = &text[..owner_end];
    let mut start = owner_end;
    let mut paren_depth = 0usize;
    for (index, ch) in prefix.char_indices().rev() {
        match ch {
            ')' => {
                paren_depth += 1;
                start = index;
            }
            '(' if paren_depth > 0 => {
                paren_depth -= 1;
                start = index;
            }
            '(' => break,
            ch if paren_depth > 0
                || ch.is_alphanumeric()
                || matches!(ch, '_' | '$' | '\\' | '-' | '>' | '?') =>
            {
                start = index;
            }
            ch if ch.is_whitespace() && paren_depth == 0 => {
                let preceding = prefix[..index].trim_end();
                if preceding.ends_with("new") {
                    start = preceding.len() - "new".len();
                }
                break;
            }
            _ => break,
        }
    }
    (start, prefix[start..].trim().to_owned())
}

pub(super) fn has_import_in_region(text: &str, fqn: &str, start: usize, end: usize) -> bool {
    text.get(start.min(text.len())..end.min(text.len()))
        .is_some_and(|region| {
            region.lines().any(|line| {
                line.trim() == format!("use {fqn};")
                    || line.trim_start().starts_with(&format!("use {fqn} as "))
            })
        })
}

pub(super) fn current_namespace(context: &str) -> String {
    context
        .lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix("namespace ")
                .map(|value| value.split(';').next().unwrap_or(value).trim().to_owned())
        })
        .unwrap_or_default()
}

pub(super) fn declared_class_fqn(context: &str) -> Option<String> {
    let declared = context.lines().find_map(|line| {
        let line = line.trim();
        // `$this` inside a trait resolves in the trait's lexical context.
        // Keep the same owner-FQN path for all PHP type declarations rather
        // than only recognizing classes.
        ["class ", "trait ", "interface ", "enum "]
            .into_iter()
            .find_map(|keyword| {
                let pos = line.find(keyword)?;
                let name = line[pos + keyword.len()..]
                    .split(|ch: char| !ch.is_alphanumeric() && ch != '_')
                    .next()?;
                (!name.is_empty()).then_some(name.to_owned())
            })
    })?;
    let namespace = current_namespace(context);
    Some(if namespace.is_empty() {
        declared
    } else {
        format!("{namespace}\\{declared}")
    })
}

pub(super) fn declared_parent_fqn(context: &str) -> Option<String> {
    let parent = context.lines().find_map(|line| {
        let line = line.trim();
        let pos = line.find("extends ")?;
        let name = line[pos + 8..]
            .split(|ch: char| !ch.is_alphanumeric() && ch != '_' && ch != '\\')
            .next()?;
        (!name.is_empty()).then_some(name.to_owned())
    })?;
    Some(resolve_php_class_name(&parent, context))
}

pub(super) fn property_type_in_context(context: &str, property: &str) -> Option<String> {
    let needle = format!("${property}");
    context.lines().find_map(|line| {
        let line = line.trim();
        let pos = line.find(&needle)?;
        let before = line[..pos].trim();
        let ty = before
            .split_whitespace()
            .last()?
            .trim_start_matches(['?', '&']);
        (ty.chars()
            .all(|ch| ch.is_alphanumeric() || matches!(ch, '_' | '\\' | '|'))
            && !ty.is_empty())
        .then(|| ty.split('|').next().unwrap_or(ty).to_owned())
    })
}


pub(super) struct CompletionPresentation {
    pub(super) primary: String,
    pub(super) return_type: Option<String>,
    pub(super) source: Option<String>,
}

pub(super) fn completion_presentation(item: &CompletionItem) -> CompletionPresentation {
    let mut detail = item.detail.clone().unwrap_or_default();
    let source = detail.rsplit_once(" • ").map(|(_, source)| {
        let source = source.trim_matches(|ch| ch == '"' || ch == '\'');
        match source {
            "PhpRuntime" | "Composer" => "Runtime".to_owned(),
            "Project" => "Project".to_owned(),
            "LSP" => "LSP".to_owned(),
            other => other.to_owned(),
        }
    });
    if let Some((head, _)) = detail.rsplit_once(" • ") {
        detail = head.to_owned();
    }
    let label = item.label.trim();
    let signature = detail.find('(').and_then(|open| {
        detail
            .rfind(')')
            .filter(|close| *close >= open)
            .map(|close| {
                (
                    detail[open + 1..close].to_owned(),
                    detail[close + 1..].trim().to_owned(),
                )
            })
    });
    let (primary, return_type) = if let Some((parameters, suffix)) = signature {
        let params = parameters
            .split(',')
            .map(str::trim)
            .filter(|param| !param.is_empty())
            .map(compress_completion_parameter)
            .collect::<Vec<_>>()
            .join(", ");
        let return_type = suffix
            .strip_prefix(':')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        (
            format!("{}({params})", label.trim_end_matches("()")),
            return_type,
        )
    } else {
        (label.to_owned(), None)
    };
    CompletionPresentation {
        primary,
        return_type,
        source,
    }
}

pub(super) fn compress_completion_parameter(parameter: &str) -> String {
    let mut value = parameter.trim().trim_start_matches('&').trim();
    value = value.strip_prefix("...").unwrap_or(value);
    value = value.split('=').next().unwrap_or(value).trim();
    value
        .rsplit_once('$')
        .map(|(_, name)| format!("${}", name.trim()))
        .unwrap_or_else(|| value.to_owned())
}

pub(super) fn completion_icon(kind: Option<CompletionItemKind>) -> &'static str {
    match kind {
        Some(CompletionItemKind::METHOD) => "M",
        Some(CompletionItemKind::FUNCTION) => "F",
        Some(CompletionItemKind::CLASS | CompletionItemKind::CONSTRUCTOR) => "C",
        Some(CompletionItemKind::INTERFACE) => "I",
        Some(CompletionItemKind::ENUM) => "E",
        Some(CompletionItemKind::STRUCT) => "S",
        Some(CompletionItemKind::PROPERTY | CompletionItemKind::FIELD) => "P",
        Some(CompletionItemKind::CONSTANT | CompletionItemKind::ENUM_MEMBER) => "#",
        _ => ".",
    }
}

/// Orders candidates already collected for a `new <prefix>` context. This is
/// deliberately a presentation-only ranking pass: it performs no lookups and
/// uses only the existing label, detail/source and kind fields.
pub(crate) fn rank_new_completion_items(items: &mut [CompletionItem], prefix: &str) {
    items.sort_by_key(|item| {
        let text_rank = if !prefix.is_empty() && item.label.eq_ignore_ascii_case(prefix) {
            0_u8 // exact short-name match
        } else if !prefix.is_empty() && starts_with_ascii_case_insensitive(&item.label, prefix) {
            1_u8 // short-name prefix match
        } else {
            2_u8 // other candidates already accepted by the provider
        };
        let source_rank = item
            .detail
            .as_deref()
            .map(|detail| {
                if detail.contains("Project") {
                    0_u8
                } else if detail.contains("Vendor") {
                    1_u8
                } else if detail.contains("Runtime") {
                    2_u8
                } else {
                    1_u8
                }
            })
            .unwrap_or(1);
        let kind_rank = match item.kind {
            Some(CompletionItemKind::CLASS)
            | Some(CompletionItemKind::CONSTRUCTOR)
            | Some(CompletionItemKind::INTERFACE)
            | Some(CompletionItemKind::ENUM) => 0_u8,
            _ => 1_u8,
        };
        (text_rank, source_rank, kind_rank)
    });
}

pub(super) fn filter_new_completion_items(items: &mut Vec<CompletionItem>, prefix: &str) {
    if prefix.is_empty() {
        return;
    }
    items.retain(|item| {
        item.label.eq_ignore_ascii_case(prefix)
            || starts_with_ascii_case_insensitive(&item.label, prefix)
    });
}

pub(super) fn starts_with_ascii_case_insensitive(value: &str, prefix: &str) -> bool {
    value
        .get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
}

impl EditorView {
    pub(super) fn navigate_completion(&mut self, delta: isize, cx: &mut Context<Self>) -> bool {
        if self.completions.is_empty() {
            return false;
        }
        let last = self.completions.len() - 1;
        self.completion_selected = if delta < 0 {
            self.completion_selected.min(last).saturating_sub(1)
        } else {
            self.completion_selected.saturating_add(1).min(last)
        };
        self.completion_scroll.scroll_to_item(self.completion_selected);
        cx.notify();
        true
    }

    pub(super) fn render_completion_popup(
        &self,
        popup_x: Pixels,
        popup_y: Pixels,
        popup_width: Pixels,
        row_height: Pixels,
        presentations: &[CompletionPresentation],
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = theme();
        let m = metrics();
                            div()
                                .absolute()
                                .left(popup_x)
                                .top(popup_y)
                                .w(popup_width)
                                .max_h(px(224.))
                                .id("completion-popup-scroll")
                                .overflow_y_scroll()
                                .track_scroll(&self.completion_scroll)
                                .rounded(m.border_radius_medium)
                                .bg(t.elevated_surface)
                                .border_1()
                                .border_color(t.border_subtle)
                                .shadow_lg()
                                .occlude()
                                .children(
                                    self.completions
                                        .iter()
                                        .enumerate()
                                        .zip(presentations.iter())
                                        .map(|((index, item), presentation)| {
                                            let editor = cx.entity();
                                            let selected = index == self.completion_selected;
                                            div()
                                                .id(("completion-item", index))
                                                .h(row_height)
                                                .w_full()
                                                .px_1()
                                                .flex()
                                                .items_center()
                                                .overflow_hidden()
                                                .on_click(move |_, _, cx| {
                                                    editor.update(cx, |this, cx| {
                                                        this.completion_selected = index;
                                                        this.accept_completion(cx);
                                                    });
                                                })
                                                .bg(if selected {
                                                    t.selection
                                                } else {
                                                    t.elevated_surface
                                                })
                                                .hover(move |style| {
                                                    style.bg(if selected {
                                                        t.selection
                                                    } else {
                                                        t.hover
                                                    })
                                                })
                                                .text_color(t.text_primary)
                                                .child(
                                                    div()
                                                        .w(px(16.0))
                                                        .flex_none()
                                                        .mr_1()
                                                        .text_color(t.info)
                                                        .child(completion_icon(item.kind)),
                                                )
                                                .child(
                                                    div()
                                                        .flex_1()
                                                        .min_w(px(0.))
                                                        .overflow_hidden()
                                                        .child(presentation.primary.clone()),
                                                )
                                                .when_some(
                                                    presentation.return_type.clone(),
                                                    |row, return_type| {
                                                        row.child(
                                                            div()
                                                                .w(px(72.))
                                                                .flex_none()
                                                                .overflow_hidden()
                                                                .ml_1()
                                                                .text_color(t.text_muted)
                                                                .child(return_type),
                                                        )
                                                    },
                                                )
                                                .when_some(
                                                    presentation.source.clone(),
                                                    |row, source| {
                                                        row.child(
                                                            div()
                                                                .w(px(64.))
                                                                .flex_none()
                                                                .overflow_hidden()
                                                                .ml_1()
                                                                .justify_end()
                                                                .text_color(t.text_muted)
                                                                .child(source),
                                                        )
                                                    },
                                                )
                                        }),
                                )
    }
}
