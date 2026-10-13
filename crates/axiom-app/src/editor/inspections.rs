use super::EditorView;
use super::*;

#[derive(Clone)]
pub(super) struct ByteDiagnostic {
    pub(super) range: Range<usize>,
    pub(super) severity: Option<DiagnosticSeverity>,
    pub(super) message: String,
}

pub(super) struct NativeInspectionResult {
    pub(super) document_session: DocumentSessionId,
    pub(super) generation: u64,
    pub(super) diagnostics: Vec<ByteDiagnostic>,
}

pub(super) struct NativeInspectionWorkItem {
    pub(super) document_session: DocumentSessionId,
    pub(super) generation: u64,
    pub(super) latest_generation: Arc<AtomicU64>,
    pub(super) unknown_class: UnknownClassInspectionInput,
    pub(super) unknown_constant: UnknownConstantInspectionInput,
    pub(super) duplicate_class: DuplicateClassInspectionInput,
    pub(super) arguments: ArgumentInspectionInput,
}

enum NativeInspectionCapture {
    Ready(NativeInspectionWorkItem),
    RetryLockBusy,
    Unavailable,
}

const NATIVE_INSPECTION_CAPTURE_RETRY_DELAY_MS: u64 = 15;
const NATIVE_INSPECTION_CAPTURE_MAX_LOCK_RETRIES: usize = 3;

pub(super) fn run_native_inspection_rules<
    UnknownClass,
    UnknownConstant,
    DuplicateClass,
    Arguments,
>(
    latest_generation: &AtomicU64,
    generation: u64,
    unknown_class: UnknownClass,
    unknown_constant: UnknownConstant,
    duplicate_class: DuplicateClass,
    arguments: Arguments,
) -> Option<Vec<ByteDiagnostic>>
where
    UnknownClass: FnOnce() -> Vec<ByteDiagnostic>,
    UnknownConstant: FnOnce() -> Vec<ByteDiagnostic>,
    DuplicateClass: FnOnce() -> Vec<ByteDiagnostic>,
    Arguments: FnOnce() -> Vec<ByteDiagnostic>,
{
    let stale = || latest_generation.load(Ordering::Acquire) != generation;
    if stale() {
        return None;
    }
    let mut diagnostics = unknown_class();
    if stale() {
        return None;
    }
    diagnostics.extend(unknown_constant());
    if stale() {
        return None;
    }
    diagnostics.extend(duplicate_class());
    if stale() {
        return None;
    }
    diagnostics.extend(arguments());
    Some(diagnostics)
}

#[derive(Clone)]
pub(super) struct UnknownClassInspectionInput {
    pub(super) text: Arc<str>,
    pub(super) project_classes: std::collections::HashSet<String>,
    pub(super) runtime_symbols: Option<Arc<RuntimeSymbolIndex>>,
    pub(super) vendor_symbols: Option<Arc<VendorSymbolIndex>>,
}

pub(super) fn compute_unknown_class_inspections(
    input: &UnknownClassInspectionInput,
    syntax: &PhpSyntax,
) -> Vec<ByteDiagnostic> {
    let mut diagnostics = Vec::new();
    let mut offset = 0;
    while let Some(relative) = input.text[offset..].find("new ") {
        let start = offset + relative + 4;
        let Some(node) = syntax
            .tree()
            .root_node()
            .descendant_for_byte_range(start, start + 1)
        else {
            break;
        };
        if node.kind() == "comment" || node.kind().contains("string") {
            offset = start + 1;
            continue;
        }
        let end = start
            + input.text[start..]
                .chars()
                .take_while(|ch| ch.is_alphanumeric() || *ch == '_' || *ch == '\\')
                .map(char::len_utf8)
                .sum::<usize>();
        if end > start {
            let written = &input.text[start..end];
            let name = written.trim_start_matches('\\');
            let resolved = resolve_php_class_name_at(written, input.text.as_ref(), start);
            let known_project =
                input.project_classes.contains(&resolved) || input.project_classes.contains(name);
            let known_runtime = input.runtime_symbols.as_ref().is_some_and(|index| {
                index.find_class(&resolved).is_some()
                    || index.find_class_by_short_name(name).is_some()
            });
            let known_vendor = input
                .vendor_symbols
                .as_ref()
                .is_some_and(|index| index.has_class_metadata(&resolved));
            if !known_project
                && !known_runtime
                && !known_vendor
                && !matches!(name, "self" | "static" | "parent")
            {
                diagnostics.push(ByteDiagnostic {
                    range: start..end,
                    severity: Some(DiagnosticSeverity::ERROR),
                    message: format!("Unknown class '{name}'"),
                });
            }
        }
        offset = end.max(start + 1);
        if offset >= input.text.len() {
            break;
        }
    }
    diagnostics
}

#[derive(Clone)]
pub(super) struct UnknownConstantInspectionInput {
    pub(super) text: Arc<str>,
    pub(super) known_constants: Vec<(String, String)>,
    pub(super) runtime_symbols: Option<Arc<RuntimeSymbolIndex>>,
}

#[derive(Clone)]
pub(super) struct DuplicateClassDeclaration {
    pub(super) fqn: String,
    pub(super) file: PersistentFileKey,
    pub(super) range: Range<usize>,
}

#[derive(Clone)]
pub(super) struct DuplicateClassInspectionInput {
    pub(super) path: PersistentFileKey,
    pub(super) declarations: Vec<DuplicateClassDeclaration>,
}

pub(super) fn compute_duplicate_class_inspections(
    input: &DuplicateClassInspectionInput,
) -> Vec<ByteDiagnostic> {
    let mut groups: HashMap<String, Vec<&DuplicateClassDeclaration>> = HashMap::new();
    for declaration in &input.declarations {
        groups
            .entry(declaration.fqn.clone())
            .or_default()
            .push(declaration);
    }
    groups
        .into_values()
        .filter(|declarations| {
            declarations
                .iter()
                .any(|declaration| declaration.file == input.path)
                && declarations
                    .iter()
                    .filter(|declaration| declaration.file != input.path)
                    .count()
                    > 0
        })
        .flat_map(|declarations| {
            declarations
                .into_iter()
                .filter(|declaration| declaration.file == input.path)
        })
        .map(|declaration| ByteDiagnostic {
            range: declaration.range.clone(),
            severity: Some(DiagnosticSeverity::ERROR),
            message: format!("Duplicate class {}", declaration.fqn),
        })
        .collect()
}

pub(super) fn compute_unknown_constant_inspections(
    input: &UnknownConstantInspectionInput,
    syntax: &PhpSyntax,
) -> Vec<ByteDiagnostic> {
    const BUILT_INS: &[&str] = &[
        "PHP_VERSION",
        "PHP_VERSION_ID",
        "PHP_INT_MAX",
        "PHP_INT_MIN",
        "PHP_INT_SIZE",
        "PHP_OS",
        "PHP_OS_FAMILY",
        "DIRECTORY_SEPARATOR",
        "PATH_SEPARATOR",
        "E_ERROR",
        "E_WARNING",
        "E_PARSE",
        "E_NOTICE",
    ];
    let mut diagnostics = Vec::new();
    let mut offset = 0;
    while let Some(relative) = input.text[offset..].find("echo ") {
        let start = offset + relative + 5;
        let Some(node) = syntax
            .tree()
            .root_node()
            .descendant_for_byte_range(start, start + 1)
        else {
            break;
        };
        if node.kind() == "comment" || node.kind().contains("string") {
            offset = start + 1;
            continue;
        }
        let end = start
            + input.text[start..]
                .chars()
                .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
                .map(char::len_utf8)
                .sum::<usize>();
        let name = &input.text[start..end];
        if !name.is_empty()
            && name
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
            && !BUILT_INS.contains(&name)
        {
            let declared = input.known_constants.iter().any(|(symbol_name, fqn)| {
                symbol_name == name || fqn.ends_with(&format!("\\{name}"))
            });
            let runtime = input
                .runtime_symbols
                .as_ref()
                .is_some_and(|symbols| symbols.find_constant(name).is_some());
            if !declared && !runtime {
                diagnostics.push(ByteDiagnostic {
                    range: start..end,
                    severity: Some(DiagnosticSeverity::WARNING),
                    message: format!("Undefined constant '{name}'"),
                });
            }
        }
        offset = end.max(start + 1);
        if offset >= input.text.len() {
            break;
        }
    }
    diagnostics
}

#[derive(Clone)]
pub(super) struct ArgumentInspectionInput {
    pub(super) text: Arc<str>,
    pub(super) project_symbols: Vec<axiom_index::ProjectSymbol>,
    pub(super) runtime_symbols: Option<Arc<RuntimeSymbolIndex>>,
    pub(super) semantic_snapshot: Option<Arc<SemanticSnapshot>>,
    pub(super) file_key: PersistentFileKey,
}

pub(super) fn compute_argument_inspections(
    input: &ArgumentInspectionInput,
    syntax: &PhpSyntax,
) -> Vec<ByteDiagnostic> {
    let mut pending = vec![syntax.tree().root_node()];
    let mut calls = Vec::new();
    while let Some(node) = pending.pop() {
        if matches!(
            node.kind(),
            "function_call_expression"
                | "member_call_expression"
                | "nullsafe_member_call_expression"
                | "static_call_expression"
                | "scoped_call_expression"
                | "object_creation_expression"
        ) {
            calls.push(node);
        }
        pending.extend(node.named_children(&mut node.walk()));
    }
    calls.sort_by_key(|node| node.start_byte());

    let mut out = Vec::new();
    for call in calls.iter() {
        let arguments_node = call.child_by_field_name("arguments").or_else(|| {
            call.named_children(&mut call.walk())
                .find(|node| node.kind() == "arguments")
        });
        let Some(arguments_node) = arguments_node else {
            continue;
        };
        let open = arguments_node.start_byte();
        if call.kind() == "object_creation_expression" {
            let class_node = call.child_by_field_name("class").or_else(|| {
                call.named_children(&mut call.walk())
                    .find(|node| matches!(node.kind(), "name" | "qualified_name"))
            });
            let Some(class_node) = class_node else {
                continue;
            };
            let class_text = class_node
                .utf8_text(input.text.as_bytes())
                .unwrap_or("")
                .trim();
            let Some(snapshot) = input.semantic_snapshot.as_ref() else {
                continue;
            };
            let Some(scope) = snapshot.scope_id_at(&input.file_key, call.start_byte()) else {
                continue;
            };
            let written = class_text;
            let Some(resolved) = snapshot.resolve_class_name(scope, written) else {
                continue;
            };
            let receiver = DeclaredType::Named {
                written: written.to_owned(),
                resolved: resolved.clone(),
            };
            let resolution = snapshot.member_resolver().resolve_method(
                scope,
                &receiver,
                "__construct",
                MemberAccess::Instance,
            );
            let MemberResolution::Resolved(id) = resolution else {
                // Missing, inaccessible, or incompatible constructors are
                // handled by their respective semantic policies, not by an
                // argument-count diagnostic.
                continue;
            };
            let Some(parameters) = snapshot
                .symbol(id)
                .and_then(|symbol| symbol.parameters.as_deref())
            else {
                continue;
            };
            let arity = signature_counts_from_detail(parameters);
            let arguments = arguments_node.named_child_count() as usize;
            if arguments < arity.required_count
                || (!arity.variadic && arguments > arity.maximum_count)
            {
                out.push(ByteDiagnostic {
                    range: arguments_node.start_byte()..arguments_node.end_byte(),
                    severity: Some(DiagnosticSeverity::ERROR),
                    message: if arguments > arity.maximum_count && !arity.variadic {
                        format!(
                            "Expected at most {} arguments, found {arguments}",
                            arity.maximum_count
                        )
                    } else {
                        format!(
                            "Expected {} argument{}, found {arguments}",
                            arity.required_count,
                            if arity.required_count == 1 { "" } else { "s" }
                        )
                    },
                });
            }
            emit_project_type_diagnostics(input, snapshot, scope, id, arguments_node, &mut out);
            continue;
        }
        if call.kind() == "function_call_expression"
            && let Some(snapshot) = input.semantic_snapshot.as_ref()
            && let Some(function) = call.child_by_field_name("function")
            && matches!(function.kind(), "name" | "qualified_name")
            && let Ok(written) = function.utf8_text(input.text.as_bytes())
            && let Some(scope) = snapshot.scope_id_at(&input.file_key, call.start_byte())
            && let Some(resolved) = snapshot.resolve_function_name(scope, written)
        {
            // Use the resolved FQN rather than an arbitrary short-name match.
            // Dynamic calls and ambiguous declarations have no safe type contract.
            let mut functions = snapshot
                .symbols_for_fqn(&resolved)
                .iter()
                .filter_map(|id| snapshot.symbol(*id))
                .filter(|symbol| symbol.kind == ProjectSymbolKind::Function);
            if let Some(function) = functions.next()
                && functions.next().is_none()
            {
                emit_project_type_diagnostics(
                    input,
                    snapshot,
                    scope,
                    function.id,
                    arguments_node,
                    &mut out,
                );
            }
        }
        let callable_start = input.text[..open]
            .char_indices()
            .rev()
            .take_while(|(_, ch)| {
                ch.is_alphanumeric() || matches!(ch, '_' | '$' | '\\' | ':' | '-' | '>')
            })
            .last()
            .map_or(open, |(i, _)| i);
        let callable = input.text[callable_start..open].trim();
        let name = callable
            .rsplit_once("::")
            .or_else(|| callable.rsplit_once("->"))
            .map(|(_, n)| n)
            .unwrap_or(callable)
            .trim_start_matches('$');
        if !name.is_empty() {
            let owner = callable
                .rsplit_once("::")
                .or_else(|| callable.rsplit_once("->"))
                .map(|(o, _)| {
                    let owner = o.trim();
                    if owner.starts_with('$') {
                        let pattern = format!("{} ", owner);
                        input.text[..open]
                            .rfind(&pattern)
                            .and_then(|pos| {
                                let before = &input.text[..pos];
                                before
                                    .rsplit(|ch: char| {
                                        !(ch.is_alphanumeric()
                                            || matches!(ch, '_' | '\\' | '?' | '|'))
                                    })
                                    .find(|part| !part.is_empty())
                                    .map(|ty| {
                                        resolve_php_class_name(
                                            ty.trim_start_matches('?'),
                                            &input.text,
                                        )
                                    })
                            })
                            .unwrap_or_else(|| resolve_php_class_name(owner, &input.text))
                    } else {
                        resolve_php_class_name(owner, &input.text)
                    }
                });
            let semantic_method = if matches!(
                call.kind(),
                "member_call_expression" | "nullsafe_member_call_expression"
            ) {
                let receiver = call
                    .child_by_field_name("object")
                    .filter(|node| node.kind() == "variable_name")
                    .and_then(|node| node.utf8_text(input.text.as_bytes()).ok())
                    .map(str::trim)
                    .map(str::to_owned);
                input.semantic_snapshot.as_ref().and_then(|snapshot| {
                    let Some(scope) = snapshot.scope_id_at(&input.file_key, call.start_byte())
                    else {
                        return None;
                    };
                    let Some(receiver) = receiver.as_deref() else {
                        return None;
                    };
                    let result = match snapshot
                        .member_resolver()
                        .resolve_binding_method(scope, receiver, name)
                    {
                        MemberResolution::Resolved(id) => Some(id),
                        _ => None,
                    };
                    result
                })
            } else {
                None
            };
            let symbol = if semantic_method.is_none() {
                owner
                    .as_deref()
                    .and_then(|o| {
                        input.project_symbols.iter().find(|s| {
                            s.kind == ProjectSymbolKind::Method
                                && s.fully_qualified_name.starts_with(&format!("{o}::"))
                                && s.name == name
                        })
                    })
                    .or_else(|| {
                        owner
                            .is_none()
                            .then(|| {
                                input.project_symbols.iter().find(|s| {
                                    s.kind == ProjectSymbolKind::Function && s.name == name
                                })
                            })
                            .flatten()
                    })
            } else {
                None
            };
            let detail = semantic_method
                .and_then(|id| input.semantic_snapshot.as_ref()?.symbol(id))
                .and_then(|s| s.parameters.as_deref())
                .map(signature_counts_from_detail)
                .or_else(|| {
                    symbol
                        .and_then(|s| s.parameters.as_deref())
                        .map(signature_counts_from_detail)
                });
            let runtime_detail = input.runtime_symbols.as_ref().and_then(|r| {
                owner
                    .as_deref()
                    .and_then(|o| r.methods_of(o).find(|s| s.name == name))
                    .and_then(|s| s.signature.as_ref())
                    .map(|s| ParameterArity {
                        required_count: s
                            .parameters
                            .iter()
                            .filter(|p| !p.optional && !p.variadic)
                            .count(),
                        maximum_count: s.parameters.len(),
                        variadic: s.parameters.iter().any(|p| p.variadic),
                    })
            });
            let counts = if semantic_method.is_some() {
                detail
            } else {
                runtime_detail.or(detail)
            };
            if let Some(arity) = counts {
                let required = arity.required_count;
                let maximum = arity.maximum_count;
                let variadic = arity.variadic;
                let arguments = arguments_node.named_child_count() as usize;
                if arguments < required || (!variadic && arguments > maximum) {
                    out.push(ByteDiagnostic {
                        range: arguments_node.start_byte()..arguments_node.end_byte(),
                        severity: Some(DiagnosticSeverity::ERROR),
                        message: if arguments > maximum && !variadic {
                            format!("Expected at most {maximum} arguments, found {arguments}")
                        } else {
                            format!(
                                "Expected {required} argument{}, found {arguments}",
                                if required == 1 { "" } else { "s" }
                            )
                        },
                    });
                }
            }
            if let (Some(id), Some(snapshot)) = (semantic_method, input.semantic_snapshot.as_ref())
            {
                if let Some(scope) = snapshot.scope_id_at(&input.file_key, call.start_byte()) {
                    emit_project_type_diagnostics(
                        input,
                        snapshot,
                        scope,
                        id,
                        arguments_node,
                        &mut out,
                    );
                }
            }
        }
    }
    if let Some(snapshot) = input.semantic_snapshot.as_ref() {
        for call in calls.iter().filter(|call| {
            matches!(
                call.kind(),
                "member_call_expression" | "nullsafe_member_call_expression"
            )
        }) {
            let (Some(object), Some(name)) = (
                call.child_by_field_name("object"),
                call.child_by_field_name("name"),
            ) else {
                continue;
            };
            let Some(scope) = snapshot.scope_id_at(&input.file_key, call.start_byte()) else {
                continue;
            };
            let resolver = axiom_index::ExpressionResolver::new(snapshot, scope);
            let Some(receiver) = resolver.infer_ast_expression_type(object, input.text.as_ref())
            else {
                continue;
            };
            if matches!(
                receiver,
                DeclaredType::Unknown(_)
                    | DeclaredType::Builtin(axiom_index::BuiltinType::Mixed)
                    | DeclaredType::Nullable(_)
            ) {
                continue;
            }
            let method_name = name.utf8_text(input.text.as_bytes()).unwrap_or("");
            let resolved = snapshot.member_resolver().resolve_method(
                scope,
                &receiver,
                method_name,
                MemberAccess::Instance,
            );
            let dynamic = matches!(
                snapshot.member_resolver().resolve_method(
                    scope,
                    &receiver,
                    "__call",
                    MemberAccess::Instance
                ),
                MemberResolution::Resolved(_) | MemberResolution::ResolvedButInaccessible(_)
            );
            if !matches!(
                resolved,
                MemberResolution::Resolved(_) | MemberResolution::ResolvedButInaccessible(_)
            ) && !dynamic
            {
                out.push(ByteDiagnostic {
                    range: name.start_byte()..name.end_byte(),
                    severity: Some(DiagnosticSeverity::ERROR),
                    message: format!("Method `{method_name}` not found"),
                });
            }
        }
    }
    out
}
#[derive(Default)]
pub(super) struct DiagnosticStore {
    pub(super) native_syntax: Vec<ByteDiagnostic>,
    pub(super) native_inspections: Vec<ByteDiagnostic>,
    pub(super) lsp: Vec<ByteDiagnostic>,
    pub(super) combined_cache: Arc<Vec<ByteDiagnostic>>,
}

impl DiagnosticStore {
    pub(super) fn rebuild(&mut self) {
        let mut diagnostics = self
            .native_syntax
            .iter()
            .chain(self.native_inspections.iter())
            .chain(self.lsp.iter())
            .cloned()
            .collect::<Vec<_>>();
        diagnostics.sort_by(|a, b| {
            a.range
                .start
                .cmp(&b.range.start)
                .then_with(|| a.range.end.cmp(&b.range.end))
                .then_with(|| {
                    diagnostic_severity_rank(a.severity).cmp(&diagnostic_severity_rank(b.severity))
                })
                .then_with(|| a.message.cmp(&b.message))
        });
        self.combined_cache = Arc::new(diagnostics);
    }

    pub(super) fn set_native_syntax(&mut self, diagnostics: Vec<ByteDiagnostic>) {
        self.native_syntax = diagnostics;
        self.rebuild();
    }

    pub(super) fn set_native_inspections(&mut self, diagnostics: Vec<ByteDiagnostic>) {
        self.native_inspections = diagnostics;
        self.rebuild();
    }

    pub(super) fn set_lsp(&mut self, diagnostics: Vec<ByteDiagnostic>) {
        self.lsp = diagnostics;
        self.rebuild();
    }

    pub(super) fn combined(&self) -> Arc<Vec<ByteDiagnostic>> {
        self.combined_cache.clone()
    }
}

pub(super) fn diagnostic_severity_rank(severity: Option<DiagnosticSeverity>) -> u8 {
    match severity {
        Some(DiagnosticSeverity::ERROR) => 0,
        Some(DiagnosticSeverity::WARNING) => 1,
        Some(DiagnosticSeverity::INFORMATION) => 2,
        Some(DiagnosticSeverity::HINT) => 3,
        Some(_) => 4,
        None => 4,
    }
}

impl EditorView {
    pub(super) fn schedule_native_inspections(&mut self, _: &str, cx: &mut Context<Self>) {
        self.native_inspection_generation = self.native_inspection_generation.wrapping_add(1);
        let generation = self.native_inspection_generation;
        self.native_inspection_latest_generation
            .store(generation, Ordering::Release);
        let session = self.document_session;
        let sender = self.native_inspection_sender.clone();
        cx.spawn(async move |this, cx| {
            gpui::Timer::after(Duration::from_millis(200)).await;
            let mut lock_retries = 0usize;
            let work = loop {
                let capture = match this.update(cx, |editor, _| {
                    if editor.document_session != session
                        || editor.native_inspection_generation != generation
                    {
                        return None;
                    }
                    let text: Arc<str> = Arc::from(editor.document.content());
                    let work = editor.capture_native_inspection_work(text, session, generation);
                    Some(work)
                }) {
                    Ok(Some(capture)) => capture,
                    Ok(None) | Err(_) => return,
                };
                match capture {
                    NativeInspectionCapture::Ready(work) => break work,
                    NativeInspectionCapture::Unavailable => return,
                    NativeInspectionCapture::RetryLockBusy => {
                        if lock_retries >= NATIVE_INSPECTION_CAPTURE_MAX_LOCK_RETRIES {
                            return;
                        }
                        lock_retries += 1;
                        gpui::Timer::after(Duration::from_millis(
                            NATIVE_INSPECTION_CAPTURE_RETRY_DELAY_MS,
                        ))
                        .await;
                    }
                }
            };
            std::thread::spawn(move || {
                let syntax = PhpSyntax::parse(work.arguments.text.as_ref()).ok();
                let Some(diagnostics) = run_native_inspection_rules(
                    &work.latest_generation,
                    work.generation,
                    || {
                        syntax.as_ref().map_or_else(Vec::new, |syntax| {
                            compute_unknown_class_inspections(&work.unknown_class, syntax)
                        })
                    },
                    || {
                        syntax.as_ref().map_or_else(Vec::new, |syntax| {
                            compute_unknown_constant_inspections(&work.unknown_constant, syntax)
                        })
                    },
                    || compute_duplicate_class_inspections(&work.duplicate_class),
                    || {
                        syntax.as_ref().map_or_else(Vec::new, |syntax| {
                            compute_argument_inspections(&work.arguments, syntax)
                        })
                    },
                ) else {
                    return;
                };
                let _ = sender.send(NativeInspectionResult {
                    document_session: work.document_session,
                    generation: work.generation,
                    diagnostics,
                });
            });
            for _ in 0..600 {
                gpui::Timer::after(Duration::from_millis(16)).await;
                if this
                    .update(cx, |editor, cx| editor.poll_native_inspection_results(cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    fn capture_native_inspection_work(
        &self,
        text: Arc<str>,
        session: DocumentSessionId,
        generation: u64,
    ) -> NativeInspectionCapture {
        let Some(project_symbols) = self.project_symbols.as_ref() else {
            return NativeInspectionCapture::Unavailable;
        };
        let index = match project_symbols.try_read() {
            Ok(index) => index,
            Err(_) => return NativeInspectionCapture::RetryLockBusy,
        };
        if !index.is_ready() {
            return NativeInspectionCapture::Unavailable;
        }
        let symbols = index.symbols().to_vec();
        let project_classes = symbols
            .iter()
            .filter(|s| {
                matches!(
                    s.kind,
                    ProjectSymbolKind::Class
                        | ProjectSymbolKind::Interface
                        | ProjectSymbolKind::Trait
                        | ProjectSymbolKind::Enum
                )
            })
            .map(|s| s.fully_qualified_name.clone())
            .collect();
        let known_constants = symbols
            .iter()
            .filter(|s| {
                matches!(
                    s.kind,
                    ProjectSymbolKind::Constant | ProjectSymbolKind::ClassConstant
                )
            })
            .map(|s| (s.name.clone(), s.fully_qualified_name.clone()))
            .collect();
        let declarations = symbols
            .iter()
            .filter(|s| {
                matches!(
                    s.kind,
                    ProjectSymbolKind::Class
                        | ProjectSymbolKind::Interface
                        | ProjectSymbolKind::Trait
                        | ProjectSymbolKind::Enum
                )
            })
            .map(|s| DuplicateClassDeclaration {
                fqn: s.fully_qualified_name.clone(),
                // ProjectSymbolIndex stores discovered files in canonical form.
                // The lexical key is therefore sufficient here and avoids a
                // filesystem call in the debounced UI capture callback.
                file: PersistentFileKey::workspace_lexical(&s.file),
                range: s.range.clone(),
            })
            .collect();
        let vendor_symbols = self
            .vendor_symbols
            .as_ref()
            .and_then(|v| v.try_read().ok())
            .map(|v| Arc::new(v.clone()));
        let semantic_snapshot = self
            .semantic_engine
            .as_ref()
            .map(|engine| engine.snapshot());
        let file_key = PersistentFileKey::workspace_lexical(&self.file_path);
        NativeInspectionCapture::Ready(NativeInspectionWorkItem {
            document_session: session,
            generation,
            latest_generation: self.native_inspection_latest_generation.clone(),
            unknown_class: UnknownClassInspectionInput {
                text: text.clone(),
                project_classes,
                runtime_symbols: self.runtime_symbols.clone(),
                vendor_symbols,
            },
            unknown_constant: UnknownConstantInspectionInput {
                text: text.clone(),
                known_constants,
                runtime_symbols: self.runtime_symbols.clone(),
            },
            duplicate_class: DuplicateClassInspectionInput {
                // The editor only schedules this work for a workspace source;
                // avoid canonicalization/filesystem access on the UI callback.
                path: PersistentFileKey::workspace_lexical(&self.file_path),
                declarations,
            },
            arguments: ArgumentInspectionInput {
                text,
                project_symbols: symbols,
                runtime_symbols: self.runtime_symbols.clone(),
                semantic_snapshot,
                file_key,
            },
        })
    }

    pub(super) fn poll_native_inspection_results(&mut self, cx: &mut Context<Self>) {
        let mut latest = None;
        while let Ok(result) = self.native_inspection_results.try_recv() {
            latest = Some(result);
        }
        if let Some(result) = latest {
            if result.document_session == self.document_session
                && result.generation == self.native_inspection_generation
            {
                self.diagnostics.set_native_inspections(result.diagnostics);
                cx.notify();
            }
        }
    }

    /// Returns whether the current document can enter the PHP completion
    /// pipeline. This is intentionally O(1) and metadata-only: no text
    /// snapshot, parsing, filesystem access, or provider lookup is involved.

    pub(super) fn syntax_is_current_for(&self, buffer_revision: u64) -> bool {
        self.syntax.is_some()
            && self
                .syntax_stamp
                .is_some_and(|stamp| stamp.buffer_revision == buffer_revision)
    }

    pub(super) fn invalidate_syntax_stamp(&mut self) {
        self.syntax_stamp = None;
    }

    pub(super) fn finish_syntax_sync(&mut self, text: &str, buffer_revision: u64, succeeded: bool) {
        let synchronized = succeeded
            && self
                .syntax
                .as_ref()
                .is_some_and(|syntax| syntax.text() == text);
        if synchronized {
            self.syntax_stamp = Some(PhpSyntaxStamp {
                buffer_revision,
                fingerprint: semantic_text_fingerprint(text),
            });
        } else {
            self.invalidate_syntax_stamp();
        }
    }

    #[allow(dead_code)]
    pub(super) fn current_syntax_fingerprint(&self) -> Option<u64> {
        let buffer_revision = self.document.buffer_revision();
        self.syntax_stamp
            .filter(|_| self.syntax_is_current_for(buffer_revision))
            .map(|stamp| stamp.fingerprint)
    }

    pub(super) fn sync_syntax(&mut self) {
        let text = self.document.content();
        let buffer_revision = self.document.buffer_revision();
        self.sync_syntax_text(&text, None, buffer_revision);
    }

    pub(super) fn sync_syntax_text(
        &mut self,
        text: &str,
        edit: Option<&DocumentEdit>,
        buffer_revision: u64,
    ) {
        let mut sync_succeeded = false;
        if let Some(syntax) = &mut self.syntax {
            let result = match edit {
                Some(edit) => match syntax
                    .apply_edit_profiled(edit.old_range_bytes.clone(), &edit.inserted_text)
                {
                    Ok(profile) if syntax.text() == text => Ok(profile),
                    // A stale or non-representable delta must not leave the
                    // resident tree out of sync; fall back to the complete
                    // document update in that rare path.
                    Ok(_) | Err(_) => syntax.update_text_profiled(text),
                },
                None => syntax.update_text_profiled(text),
            };
            match result {
                Ok(_) => sync_succeeded = true,
                Err(error) => {
                    self.status = Some(format!("Falha ao atualizar sintaxe PHP: {error}").into())
                }
            }
        }
        self.finish_syntax_sync(text, buffer_revision, sync_succeeded);
        let diagnostics = self
            .syntax
            .as_ref()
            .map(|syntax| {
                syntax
                    .diagnostics()
                    .into_iter()
                    .map(|diagnostic| ByteDiagnostic {
                        range: diagnostic.range,
                        severity: Some(DiagnosticSeverity::ERROR),
                        message: diagnostic.message,
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.diagnostics.set_native_syntax(diagnostics);
    }

    pub fn set_diagnostics(
        &mut self,
        version: Option<i32>,
        diagnostics: Vec<Diagnostic>,
        cx: &mut Context<Self>,
    ) {
        if version.is_some_and(|version| version < self.lsp_version) {
            return;
        }
        self.diagnostics.set_lsp(
            diagnostics
                .into_iter()
                .map(|diagnostic| ByteDiagnostic {
                    range: self.lsp_range_to_bytes(diagnostic.range),
                    severity: diagnostic.severity,
                    message: diagnostic.message,
                })
                .collect(),
        );
        cx.notify();
    }

    pub(super) fn render_line(
        &self,
        line: usize,
        window: &mut Window,
        diagnostics: &[ByteDiagnostic],
    ) -> gpui::AnyElement {
        let t = theme();
        let raw = self.document.line_content(line);
        let text = trim_eol(raw.as_ref()).to_owned();
        let shaped_width = self.line_layout(line, &text, window).width;
        let start = self.document.offset_of_line(line);
        let end = start + text.len();
        let selection = self.document.selection_offsets();
        let cursor = self.document.cursor_offset();
        let cursor_here = self.document.line_of_offset(cursor) == line;
        let selected_start = selection.map_or(end, |(a, _)| a.max(start).min(end)) - start;
        let selected_end = selection.map_or(end, |(_, b)| b.max(start).min(end)) - start;
        let cursor_column = cursor.saturating_sub(start).min(text.len());
        let hover = self.ctrl_hover_range.as_ref().and_then(|range| {
            (range.end > start && range.start < end).then(|| {
                let from = range.start.max(start) - start;
                let to = range.end.min(end) - start;
                let layout = self.line_layout(line, &text, window);
                (layout.x_for_index(from), layout.x_for_index(to))
            })
        });
        let highlights: Vec<_> = self
            .syntax
            .as_ref()
            .into_iter()
            .flat_map(|syntax| syntax.highlights_in(start..end))
            .cloned()
            .collect();
        // Render at most one marker per line, choosing the most severe
        // diagnostic when several ranges overlap this line.
        let diagnostic = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.range.end > start && diagnostic.range.start < end)
            .min_by_key(|diagnostic| diagnostic_severity_rank(diagnostic.severity));
        let diagnostic_underline = diagnostic.map(|diagnostic| {
            let from = diagnostic.range.start.max(start) - start;
            let to = diagnostic.range.end.min(end) - start;
            let layout = self.line_layout(line, &text, window);
            (
                layout.x_for_index(from),
                layout.x_for_index(to),
                diagnostic.severity,
            )
        });
        let content = if selected_start < selected_end {
            div()
                .flex()
                .h_full()
                .child(styled_segment(
                    text[..selected_start].to_owned(),
                    start,
                    &highlights,
                ))
                .child(div().h_full().bg(t.selection).child(styled_segment(
                    text[selected_start..selected_end].to_owned(),
                    start + selected_start,
                    &highlights,
                )))
                .child(styled_segment(
                    text[selected_end..].to_owned(),
                    start + selected_end,
                    &highlights,
                ))
        } else if cursor_here && self.caret_visible && self.focus.is_focused(window) {
            let layout = self.line_layout(line, &text, window);
            let caret_x = layout.x_for_index(cursor_column);
            let metrics = shape(window, "M");
            let caret_height = (metrics.ascent + metrics.descent).min(px(LINE_HEIGHT));
            div()
                .relative()
                .h_full()
                .child(styled_segment(text, start, &highlights))
                .child(
                    div()
                        .absolute()
                        .left(caret_x)
                        .top((px(LINE_HEIGHT) - caret_height) / 2.)
                        .w(px(1.))
                        .h(caret_height)
                        .bg(t.text_primary),
                )
        } else {
            div()
                .flex()
                .h_full()
                .child(styled_segment(text, start, &highlights))
        };

        div()
            .id(line)
            .relative()
            .flex()
            .w(self
                .content_width
                .max(px(GUTTER_WIDTH + TEXT_PADDING) + shaped_width)
                + px(HORIZONTAL_END_PADDING))
            .h(px(LINE_HEIGHT))
            .line_height(px(LINE_HEIGHT))
            .text_size(px(FONT_SIZE))
            .font_family(CODE_FONT_FAMILY)
            .text_color(t.text_primary)
            .bg(if self.navigation_highlight_line == Some(line) {
                t.inactive_selection
            } else if cursor_here {
                t.active_line
            } else {
                t.editor_background
            })
            .child(
                div()
                    .w(px(GUTTER_WIDTH))
                    .pr_3()
                    .bg(t.gutter_background)
                    .border_r_1()
                    .border_color(t.border_subtle)
                    .cursor(CursorStyle::Arrow)
                    .text_right()
                    .text_color(
                        if diagnostic.is_some_and(|diagnostic| {
                            diagnostic.severity == Some(DiagnosticSeverity::ERROR)
                        }) {
                            t.error
                        } else if diagnostic.is_some() {
                            t.warning
                        } else if cursor_here {
                            t.text_secondary
                        } else {
                            t.gutter_text
                        },
                    )
                    .child(format!(
                        "{}{}",
                        if diagnostic.is_some() { "● " } else { "" },
                        line + 1
                    )),
            )
            // Keep the code column at its shaped text width so the
            // unconstrained uniform list can expose horizontal scrolling.
            .child(div().flex_none().h_full().pl_3().child(content))
            .when_some(hover, |this, (from, to)| {
                this.child(
                    div()
                        .absolute()
                        .left(px(GUTTER_WIDTH) + px(TEXT_PADDING) + from)
                        .top(px(LINE_HEIGHT - 2.))
                        .w((to - from).max(px(1.)))
                        .h(px(1.))
                        .bg(t.accent),
                )
            })
            .when_some(diagnostic_underline, |this, (from, to, severity)| {
                let color = match severity {
                    Some(DiagnosticSeverity::ERROR) => t.error,
                    Some(DiagnosticSeverity::WARNING) => t.warning,
                    Some(DiagnosticSeverity::INFORMATION) => t.info,
                    Some(DiagnosticSeverity::HINT) => t.accent,
                    _ => t.warning,
                };
                let width: f32 = (to - from).max(px(1.)).into();
                let wave_step = 2.0_f32;
                let segment_count = ((width / wave_step).ceil() as usize).max(4);
                this.child(
                    div()
                        .absolute()
                        .left(px(GUTTER_WIDTH) + px(TEXT_PADDING) + from)
                        .top(px(LINE_HEIGHT - 4.))
                        .w(px(width))
                        .h(px(4.))
                        .overflow_hidden()
                        .children((0..segment_count).map(move |index| {
                            let phase = index % 4;
                            div()
                                .absolute()
                                .left(px(index as f32 * wave_step))
                                .top(match phase {
                                    0 | 2 => px(1.),
                                    1 => px(0.),
                                    _ => px(2.),
                                })
                                .w(px(wave_step + 1.))
                                .h(px(0.5))
                                .bg(color)
                        })),
                )
            })
            .into_any_element()
    }

    pub(super) fn update_diagnostic_hover(
        &mut self,
        line: usize,
        offset: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let next = self
            .diagnostics
            .combined()
            .iter()
            .find(|diagnostic| diagnostic.range.contains(&offset))
            .map(|diagnostic| format!("{}\nAxiom PHP Parser", diagnostic.message));
        if self.hover_popup == next {
            return;
        }
        self.hover_popup = next;
        self.hover_anchor = self.hover_popup.as_ref().map(|_| {
            let line_start = self.document.offset_of_line(line);
            let raw_line = self.document.line_content(line);
            let line_text = trim_eol(raw_line.as_ref());
            let column = offset.saturating_sub(line_start).min(line_text.len());
            let caret_x = self
                .line_layout(line, line_text, window)
                .x_for_index(column);
            let scroll_y = self.scroll.0.borrow().base_handle.offset().y;
            gpui::point(
                px(GUTTER_WIDTH + TEXT_PADDING) + caret_x,
                px(line as f32 * LINE_HEIGHT) - scroll_y,
            )
        });
        cx.notify();
    }
}
pub(super) fn emit_project_type_diagnostics(
    input: &ArgumentInspectionInput,
    snapshot: &SemanticSnapshot,
    scope: axiom_index::ScopeId,
    symbol_id: axiom_index::SymbolId,
    arguments_node: tree_sitter::Node<'_>,
    out: &mut Vec<ByteDiagnostic>,
) {
    let Some(symbol) = snapshot.symbol(symbol_id) else {
        return;
    };
    if symbol.structured_parameters.is_empty() {
        return;
    }
    let resolver = axiom_index::ExpressionResolver::new(snapshot, scope);
    let parameters = &symbol.structured_parameters;
    let mut parameter_index = 0usize;
    for argument in arguments_node.named_children(&mut arguments_node.walk()) {
        // Named arguments and unpacking need dedicated mapping semantics; do
        // not guess at their parameter or type in this conservative phase.
        if argument.kind() == "named_argument" || argument.kind() == "variadic_unpacking" {
            parameter_index += 1;
            continue;
        }
        let Some(parameter) = parameters
            .get(parameter_index)
            .or_else(|| parameters.last().filter(|parameter| parameter.variadic))
        else {
            break;
        };
        if let Some(expected) = parameter.declared_type.as_ref() {
            if let Some(actual) = resolver.infer_ast_expression_type(argument, input.text.as_ref())
            {
                let compatibility = declared_type_compatibility(snapshot, expected, &actual);
                if compatibility == TypeCompatibility::Incompatible {
                    out.push(ByteDiagnostic {
                        range: argument.start_byte()..argument.end_byte(),
                        severity: Some(DiagnosticSeverity::ERROR),
                        message: format!(
                            "Expected {}, found {}",
                            declared_type_label(expected),
                            declared_type_label(&actual)
                        ),
                    });
                }
            }
        }
        if !parameter.variadic {
            parameter_index += 1;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ParameterArity {
    pub(super) required_count: usize,
    pub(super) maximum_count: usize,
    pub(super) variadic: bool,
}

pub(super) fn signature_counts_from_detail(detail: &str) -> ParameterArity {
    let Some(open) = detail.find('(') else {
        return ParameterArity {
            required_count: 0,
            maximum_count: 0,
            variadic: false,
        };
    };
    let Some(close) = matching_paren(detail, open) else {
        return ParameterArity {
            required_count: 0,
            maximum_count: 0,
            variadic: false,
        };
    };
    let parameters = &detail[open + 1..close];
    if parameters.trim().is_empty() {
        return ParameterArity {
            required_count: 0,
            maximum_count: 0,
            variadic: false,
        };
    }
    let mut required = 0;
    let mut maximum = 0;
    let mut variadic = false;
    for parameter in split_signature_parameters(parameters) {
        let parameter = parameter.trim();
        if parameter.is_empty() {
            continue;
        }
        maximum += 1;
        if parameter.contains("...") {
            variadic = true;
        }
        if !parameter.contains('=') && !parameter.contains("...") {
            required += 1;
        }
    }
    ParameterArity {
        required_count: required,
        maximum_count: maximum,
        variadic,
    }
}

/// Splits a PHP parameter list only at top-level commas. Types and defaults
/// may contain nested arrays/calls and quoted strings with commas.
pub(super) fn split_signature_parameters(parameters: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut nesting = 0usize;
    let mut quote = None;
    let mut escaped = false;
    for (offset, ch) in parameters.char_indices() {
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == active_quote {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            '(' | '[' | '{' => nesting += 1,
            ')' | ']' | '}' => nesting = nesting.saturating_sub(1),
            ',' if nesting == 0 => {
                result.push(&parameters[start..offset]);
                start = offset + ch.len_utf8();
            }
            _ => {}
        }
    }
    result.push(&parameters[start..]);
    result
}
