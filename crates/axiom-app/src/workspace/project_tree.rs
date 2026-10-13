use super::*;

impl WorkspaceView {
    pub(super) fn toggle_directory(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(item) = self.explorer.get(index).cloned() else {
            return;
        };
        if item.kind != EntryKind::Directory {
            return;
        }
        if self.expanded.remove(&item.path) {
            let prefix = item.path;
            self.explorer.retain(|candidate| {
                candidate.path == prefix || !candidate.path.starts_with(&prefix)
            });
        } else if let Some(project) = self.project.clone() {
            let directory = item.path.clone();
            let workspace = cx.entity().downgrade();
            cx.spawn(async move |_, cx| {
                let result = project.read_directory(&directory);
                let _ = workspace.update(cx, |this, cx| {
                    match result {
                        Ok(entries) => {
                            let Some(current_index) = this
                                .explorer
                                .iter()
                                .position(|candidate| candidate.path == directory)
                            else {
                                return;
                            };
                            if this.expanded.contains(&directory) {
                                return;
                            }
                            let insert_at = current_index + 1;
                            if this
                                .explorer
                                .iter()
                                .skip(insert_at)
                                .any(|candidate| candidate.path.starts_with(&directory))
                            {
                                this.expanded.insert(directory.clone());
                                return;
                            }
                            let depth = this.explorer[current_index].depth + 1;
                            let children = entries.into_iter().map(|entry| ExplorerItem {
                                path: entry.path,
                                name: entry.name,
                                kind: entry.kind,
                                depth,
                            });
                            this.explorer.splice(insert_at..insert_at, children);
                            this.expanded.insert(directory.clone());
                        }
                        Err(error) => this.status = format!("Falha ao abrir pasta: {error}").into(),
                    }
                    cx.notify();
                });
            })
            .detach();
        }
        cx.notify();
    }

    pub(super) fn refresh_explorer(&mut self, cx: &mut Context<Self>) {
        let Some(project) = self.project.clone() else {
            return;
        };
        let root = project.root_path().to_path_buf();
        let expanded = self.expanded.clone();
        self.explorer_refresh_generation = self.explorer_refresh_generation.wrapping_add(1);
        self.explorer_structure_generation = self.explorer_structure_generation.wrapping_add(1);
        self.explorer_structure_snapshot = None;
        let refresh_generation = self.explorer_refresh_generation;
        let git_scan_inflight = self.git_scan_inflight.clone();
        let workspace = cx.entity().downgrade();
        self.status = "Refreshing Project Explorer...".into();
        self.explorer_context = None;
        cx.notify();
        cx.spawn(async move |_, cx| {
            let result = (|| {
                let entries = project
                    .read_directory(&root)
                    .map_err(|error| error.to_string())?;
                let mut visible: Vec<ExplorerItem> = entries
                    .into_iter()
                    .map(|entry| ExplorerItem {
                        path: entry.path,
                        name: entry.name,
                        kind: entry.kind,
                        depth: 0,
                    })
                    .collect();
                let mut expanded_paths: Vec<_> = expanded.into_iter().collect();
                expanded_paths.sort_by_key(|path| path.components().count());
                let mut valid_expanded = HashSet::new();
                for directory in expanded_paths {
                    let Some(index) = visible.iter().position(|item| item.path == directory) else {
                        continue;
                    };
                    if visible[index].kind != EntryKind::Directory {
                        continue;
                    }
                    let children = match project.read_directory(&directory) {
                        Ok(children) => children,
                        Err(_) => continue,
                    };
                    let depth = visible[index].depth + 1;
                    let child_items = children.into_iter().map(|entry| ExplorerItem {
                        path: entry.path,
                        name: entry.name,
                        kind: entry.kind,
                        depth,
                    });
                    let insert_at = index + 1;
                    visible.splice(insert_at..insert_at, child_items);
                    valid_expanded.insert(directory);
                }
                let git_state = load_git_state(&root, &git_scan_inflight);
                Ok::<_, String>((visible, valid_expanded, git_state))
            })();
            let _ = workspace.update(cx, |this, cx| {
                if this.explorer_refresh_generation != refresh_generation {
                    return;
                }
                match result {
                    Ok((items, expanded, git_state)) => {
                        this.explorer = items;
                        this.expanded = expanded;
                        if let Ok(git_state) = git_state {
                            this.git_state = git_state;
                        }
                        this.status = "Project Explorer refreshed".into();
                    }
                    Err(error) => {
                        this.status = format!("Falha ao atualizar projeto: {error}").into()
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn context_directory(path: &Path, kind: EntryKind) -> &Path {
        if kind == EntryKind::Directory {
            path
        } else {
            path.parent().unwrap_or(path)
        }
    }

    pub(super) fn close_context_menu(&mut self, reason: &'static str, cx: &mut Context<Self>) {
        if self.explorer_context.take().is_some() {
            self.explorer_new_menu_open = false;
            if debug_input_enabled() {
                tracing::info!(
                    selected_path = ?self.selected_path,
                    reason,
                    "[CONTEXT MENU CLOSE]"
                );
            }
            cx.notify();
        }
    }

    pub(super) fn open_context_menu(
        &mut self,
        path: PathBuf,
        kind: EntryKind,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.selected_path = Some(path.clone());
        self.context_menu_position = position;
        self.open_menu = None;
        self.context_menu_selected = 0;
        self.context_submenu_selected = 0;
        self.explorer_new_menu_open = false;
        self.explorer_context = Some(ExplorerContext { path, kind });
        if debug_input_enabled() {
            tracing::info!(selected_path = ?self.selected_path, "[CONTEXT MENU OPEN]");
        }
        cx.notify();
    }

    pub(super) fn open_context_submenu(&mut self, cx: &mut Context<Self>) {
        if self.explorer_context.is_some() {
            self.explorer_new_menu_open = true;
            self.context_submenu_selected = 0;
            if debug_input_enabled() {
                tracing::info!(selected_path = ?self.selected_path, "[SUBMENU OPEN]");
            }
            cx.notify();
        }
    }

    pub(super) fn execute_new_submenu_item(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(context) = self.explorer_context.as_ref() else {
            return;
        };
        let directory = Self::context_directory(&context.path, context.kind).to_path_buf();
        if debug_input_enabled() {
            tracing::info!(
                selected_path = ?self.selected_path,
                index = self.context_submenu_selected,
                "[CONTEXT MENU ACTION]"
            );
            let kind = [
                "file",
                "directory",
                "php_file",
                "php_class",
                "php_interface",
                "php_trait",
                "php_enum",
            ]
            .get(self.context_submenu_selected)
            .copied()
            .unwrap_or("unknown");
            tracing::info!(kind, selected_path = ?self.selected_path, target_directory = %directory.display(), "[NEW ITEM ACTION]");
        }
        let kind = match self.context_submenu_selected {
            0 => NewItemKind::File,
            1 => NewItemKind::Directory,
            2 => NewItemKind::PhpFile,
            3 => NewItemKind::PhpClass,
            4 => NewItemKind::PhpInterface,
            5 => NewItemKind::PhpTrait,
            6 => NewItemKind::PhpEnum,
            _ => return,
        };
        self.begin_new_item(kind, directory, window, cx);
    }

    pub(super) fn new_file(&mut self, directory: PathBuf, _: &mut Window, cx: &mut Context<Self>) {
        if self.explorer_fs_busy {
            self.status = "Another file operation is in progress".into();
            cx.notify();
            return;
        }
        if self.explorer_context.is_some() && debug_input_enabled() {
            tracing::info!(selected_path = ?self.selected_path, reason = "action", "[CONTEXT MENU CLOSE]");
        }
        self.explorer_context = None;
        self.explorer_new_menu_open = false;
        self.explorer_undo.clear();
        self.explorer_input = "untitled".into();
        self.explorer_selection = UTF16Selection {
            range: 0..self.explorer_input.encode_utf16().count(),
            reversed: false,
        };
        self.explorer_modal_field = ModalField::Name;
        self.modal_focus_pending = true;
        self.explorer_namespace.clear();
        self.explorer_operation = Some(ExplorerOperation::NewFile(directory));
        cx.notify();
    }

    pub(super) fn begin_new_item(
        &mut self,
        kind: NewItemKind,
        directory: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if debug_input_enabled() {
            tracing::info!(?kind, target_directory = %directory.display(), "[NEW ITEM ACTION]");
        }
        match kind {
            NewItemKind::File => self.new_file(directory, window, cx),
            NewItemKind::Directory => self.new_directory(directory, cx),
            NewItemKind::PhpFile => self.new_php_file(directory, cx),
            NewItemKind::PhpClass => self.new_php_item(directory, "class", cx),
            NewItemKind::PhpInterface => self.new_php_item(directory, "interface", cx),
            NewItemKind::PhpTrait => self.new_php_item(directory, "trait", cx),
            NewItemKind::PhpEnum => self.new_php_item(directory, "enum", cx),
        }
    }

    pub(super) fn open_new_menu(&mut self, directory: PathBuf, cx: &mut Context<Self>) {
        self.selected_path = Some(directory.clone());
        self.context_menu_position = Point::new(px(34.), px(70.));
        self.explorer_context = Some(ExplorerContext {
            path: directory,
            kind: EntryKind::Directory,
        });
        self.explorer_new_menu_open = true;
        if debug_input_enabled() {
            tracing::info!("[EXPLORER TOOLBAR CLICK] button=new");
            tracing::info!(command = "project.new", "[ACTION]");
        }
        cx.notify();
    }

    pub(super) fn new_php_file(&mut self, directory: PathBuf, cx: &mut Context<Self>) {
        if self.explorer_fs_busy {
            self.status = "Another file operation is in progress".into();
            cx.notify();
            return;
        }
        if self.explorer_context.is_some() && debug_input_enabled() {
            tracing::info!(selected_path = ?self.selected_path, reason = "action", "[CONTEXT MENU CLOSE]");
        }
        self.explorer_context = None;
        self.explorer_new_menu_open = false;
        self.explorer_undo.clear();
        self.explorer_input = "untitled.php".into();
        self.explorer_selection = UTF16Selection {
            range: 0..self.explorer_input.encode_utf16().count(),
            reversed: false,
        };
        self.explorer_modal_field = ModalField::Name;
        self.modal_focus_pending = true;
        self.explorer_namespace.clear();
        self.explorer_operation = Some(ExplorerOperation::NewPhpFile(directory));
        cx.notify();
    }

    pub(super) fn new_directory(&mut self, directory: PathBuf, cx: &mut Context<Self>) {
        if self.explorer_fs_busy {
            self.status = "Another file operation is in progress".into();
            cx.notify();
            return;
        }
        if self.explorer_context.is_some() && debug_input_enabled() {
            tracing::info!(selected_path = ?self.selected_path, reason = "action", "[CONTEXT MENU CLOSE]");
        }
        self.explorer_context = None;
        self.explorer_new_menu_open = false;
        self.explorer_undo.clear();
        self.explorer_input = "New Folder".into();
        self.explorer_selection = UTF16Selection {
            range: 0..self.explorer_input.encode_utf16().count(),
            reversed: false,
        };
        self.explorer_modal_field = ModalField::Name;
        self.modal_focus_pending = true;
        self.explorer_namespace.clear();
        self.explorer_operation = Some(ExplorerOperation::NewDirectory(directory));
        cx.notify();
    }

    pub(super) fn new_php_item(
        &mut self,
        directory: PathBuf,
        keyword: &'static str,
        cx: &mut Context<Self>,
    ) {
        self.modal_type_items.clear();
        if self.explorer_fs_busy {
            self.status = "Another file operation is in progress".into();
            cx.notify();
            return;
        }
        if self.explorer_context.is_some() && debug_input_enabled() {
            tracing::info!(selected_path = ?self.selected_path, reason = "action", "[CONTEXT MENU CLOSE]");
        }
        self.explorer_context = None;
        self.explorer_new_menu_open = false;
        self.explorer_undo.clear();
        self.explorer_input = "NewItem".into();
        self.explorer_file = "NewItem.php".into();
        self.explorer_file_auto = true;
        self.explorer_selection = UTF16Selection {
            range: 0..self.explorer_input.encode_utf16().count(),
            reversed: false,
        };
        self.explorer_modal_field = ModalField::Name;
        self.modal_focus_pending = true;
        self.explorer_namespace = self
            .project
            .as_ref()
            .and_then(|project| project.path_to_namespace(&directory))
            .unwrap_or_default();
        self.explorer_extends.clear();
        self.explorer_implements.clear();
        self.explorer_operation = Some(ExplorerOperation::NewPhp { directory, keyword });
        cx.notify();
    }

    pub(super) fn select_php_type(&mut self, keyword: &'static str, cx: &mut Context<Self>) {
        self.modal_type_items.clear();
        if let Some(ExplorerOperation::NewPhp {
            keyword: current, ..
        }) = self.explorer_operation.as_mut()
        {
            *current = keyword;
            if !matches!(keyword, "class" | "interface") {
                self.explorer_extends.clear();
                self.explorer_implements.clear();
            }
            if (self.explorer_modal_field == ModalField::Implements && keyword != "class")
                || (self.explorer_modal_field == ModalField::Extends
                    && !matches!(keyword, "class" | "interface"))
            {
                self.set_modal_field(ModalField::Name);
            }
            cx.notify();
        }
    }

    pub(super) fn modal_type_context(&self) -> Option<crate::editor_view::TypeCompletionContext> {
        use crate::editor_view::TypeCompletionContext::*;
        match (&self.explorer_operation, self.explorer_modal_field) {
            (
                Some(ExplorerOperation::NewPhp {
                    keyword: "class", ..
                }),
                ModalField::Extends,
            ) => Some(ClassExtends),
            (
                Some(ExplorerOperation::NewPhp {
                    keyword: "class", ..
                }),
                ModalField::Implements,
            ) => Some(ClassImplements),
            (
                Some(ExplorerOperation::NewPhp {
                    keyword: "interface",
                    ..
                }),
                ModalField::Extends,
            ) => Some(InterfaceExtends),
            _ => None,
        }
    }

    pub(super) fn refresh_modal_types(&mut self) {
        self.modal_type_items.clear();
        let Some(context) = self.modal_type_context() else {
            return;
        };
        let field = self.explorer_modal_field;
        let text = self.modal_field_text(field);
        let caret = utf16_to_byte_offset(text, self.modal_field_selection(field).range.end);
        let (range, prefix) = crate::modal_type_completion::active_token(text, caret);
        // A busy index is skipped; never wait on the UI thread.
        let project = self
            .project_index
            .as_ref()
            .and_then(|index| index.try_read().ok());
        let vendor = self
            .vendor_index
            .as_ref()
            .and_then(|index| index.try_read().ok());
        let items = crate::modal_type_completion::lookup(
            context,
            prefix,
            project.as_deref(),
            vendor.as_deref(),
            self._runtime_symbols.as_deref(),
        );
        self.modal_type_items = items;
        self.modal_type_range = range;
        self.modal_type_selected = 0;
        self.modal_type_scroll
            .set_offset(gpui::point(px(0.), px(0.)));
    }

    pub(super) fn accept_modal_type(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.modal_type_context().is_none() {
            return;
        }
        let Some(fqn) = self
            .modal_type_items
            .get(index)
            .and_then(|item| item.insert_text.clone())
        else {
            return;
        };
        let field = self.explorer_modal_field;
        let text = self.modal_field_text(field);
        let range = byte_to_utf16_offset(text, self.modal_type_range.start)
            ..byte_to_utf16_offset(text, self.modal_type_range.end);
        self.modal_replace_field_range(field, range, &fqn, cx);
        self.modal_type_items.clear();
        cx.notify();
    }

    pub(super) fn modal_completion_key(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        if self.modal_type_items.is_empty() {
            return false;
        }
        match key {
            "up" => {
                self.modal_type_selected = (self.modal_type_selected + self.modal_type_items.len()
                    - 1)
                    % self.modal_type_items.len()
            }
            "down" => {
                self.modal_type_selected =
                    (self.modal_type_selected + 1) % self.modal_type_items.len()
            }
            "enter" => {
                self.accept_modal_type(self.modal_type_selected, cx);
                return true;
            }
            "escape" => self.modal_type_items.clear(),
            "tab" => {
                self.modal_type_items.clear();
                return false;
            }
            _ => return false,
        }
        self.modal_type_scroll
            .scroll_to_item(self.modal_type_selected);
        cx.notify();
        true
    }

    pub(super) fn cycle_modal_field(&mut self, backwards: bool) {
        let fields = match self.explorer_operation {
            Some(ExplorerOperation::NewPhp {
                keyword: "class", ..
            }) => vec![
                ModalField::Name,
                ModalField::Namespace,
                ModalField::File,
                ModalField::Extends,
                ModalField::Implements,
            ],
            Some(ExplorerOperation::NewPhp {
                keyword: "interface",
                ..
            }) => vec![
                ModalField::Name,
                ModalField::Namespace,
                ModalField::File,
                ModalField::Extends,
            ],
            Some(ExplorerOperation::NewPhp { .. }) => {
                vec![ModalField::Name, ModalField::Namespace, ModalField::File]
            }
            _ => vec![ModalField::Name],
        };
        let index = fields
            .iter()
            .position(|field| *field == self.explorer_modal_field)
            .unwrap_or(0);
        let next = if backwards {
            index.checked_sub(1).unwrap_or(fields.len() - 1)
        } else {
            (index + 1) % fields.len()
        };
        self.set_modal_field(fields[next]);
    }

    pub(super) fn rename_entry(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self.explorer_fs_busy {
            self.status = "Another file operation is in progress".into();
            cx.notify();
            return;
        }
        if debug_input_enabled() {
            tracing::info!(selected_path = %path.display(), popup_open = true, "[RENAME DIALOG]");
        }
        self.explorer_context = None;
        self.explorer_new_menu_open = false;
        let Some(current_name) = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_owned)
        else {
            return;
        };
        self.explorer_input = current_name;
        self.explorer_undo.clear();
        let basename_len = self
            .explorer_input
            .rsplit_once('.')
            .filter(|(_, extension)| !extension.is_empty())
            .map(|(basename, _)| basename.encode_utf16().count())
            .unwrap_or_else(|| self.explorer_input.encode_utf16().count());
        self.explorer_selection = UTF16Selection {
            range: 0..basename_len,
            reversed: false,
        };
        self.explorer_modal_field = ModalField::Name;
        self.modal_focus_pending = true;
        self.explorer_namespace.clear();
        self.explorer_operation = Some(ExplorerOperation::Rename(path));
        cx.notify();
    }

    pub(super) fn cancel_explorer_operation(&mut self, cx: &mut Context<Self>) {
        self.modal_type_items.clear();
        self.explorer_operation = None;
        self.explorer_new_menu_open = false;
        self.modal_focus_pending = false;
        self.explorer_input.clear();
        self.explorer_file.clear();
        self.explorer_file_auto = true;
        self.explorer_undo.clear();
        self.explorer_namespace.clear();
        self.explorer_extends.clear();
        self.explorer_implements.clear();
        cx.notify();
    }

    pub(super) fn modal_field_text(&self, field: ModalField) -> &str {
        match field {
            ModalField::Name => &self.explorer_input,
            ModalField::Namespace => &self.explorer_namespace,
            ModalField::File => &self.explorer_file,
            ModalField::Extends => &self.explorer_extends,
            ModalField::Implements => &self.explorer_implements,
        }
    }

    pub(super) fn modal_field_text_mut(&mut self, field: ModalField) -> &mut String {
        match field {
            ModalField::Name => &mut self.explorer_input,
            ModalField::Namespace => &mut self.explorer_namespace,
            ModalField::File => &mut self.explorer_file,
            ModalField::Extends => &mut self.explorer_extends,
            ModalField::Implements => &mut self.explorer_implements,
        }
    }

    pub(super) fn modal_field_selection(&self, field: ModalField) -> &UTF16Selection {
        match field {
            ModalField::Name => &self.explorer_selection,
            ModalField::Namespace => &self.explorer_namespace_selection,
            ModalField::File => &self.explorer_file_selection,
            ModalField::Extends => &self.explorer_extends_selection,
            ModalField::Implements => &self.explorer_implements_selection,
        }
    }

    pub(super) fn modal_field_selection_mut(&mut self, field: ModalField) -> &mut UTF16Selection {
        match field {
            ModalField::Name => &mut self.explorer_selection,
            ModalField::Namespace => &mut self.explorer_namespace_selection,
            ModalField::File => &mut self.explorer_file_selection,
            ModalField::Extends => &mut self.explorer_extends_selection,
            ModalField::Implements => &mut self.explorer_implements_selection,
        }
    }

    pub(super) fn set_modal_field(&mut self, field: ModalField) {
        self.modal_type_items.clear();
        self.explorer_modal_field = field;
        self.reset_modal_caret();
    }

    pub(super) fn modal_caret_for(&self, field: ModalField, window: &Window) -> bool {
        self.modal_caret_visible
            && self.explorer_modal_field == field
            && self.modal_field_focus[field as usize].is_focused(window)
    }

    pub(super) fn set_modal_field_at(&mut self, field: ModalField, byte: usize) {
        self.set_modal_field(field);
        let caret = byte_to_utf16_offset(
            self.modal_field_text(field),
            byte.min(self.modal_field_text(field).len()),
        );
        *self.modal_field_selection_mut(field) = UTF16Selection {
            range: caret..caret,
            reversed: false,
        };
    }

    pub(super) fn modal_pointer_down(
        &mut self,
        field: ModalField,
        event: &gpui::MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let byte = self.modal_field_geometry[field as usize].hit_test(event.position.x);
        self.set_modal_field_at(field, byte);
        if event.click_count == 2 {
            let text = self.modal_field_text(field);
            let range = axiom_app::interaction::word_range_at(text, byte);
            let range =
                byte_to_utf16_offset(text, range.start)..byte_to_utf16_offset(text, range.end);
            self.modal_field_selection_mut(field).range = range;
        }
        window.focus(&self.modal_field_focus[field as usize]);
        cx.notify();
    }

    pub(super) fn modal_pointer_move(
        &mut self,
        field: ModalField,
        event: &gpui::MouseMoveEvent,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        if !event.dragging()
            || self.explorer_modal_field != field
            || !self.modal_field_focus[field as usize].is_focused(window)
        {
            return;
        }
        let byte = self.modal_field_geometry[field as usize].hit_test(event.position.x);
        let caret = byte_to_utf16_offset(self.modal_field_text(field), byte);
        let selection = self.modal_field_selection_mut(field);
        let anchor = if selection.reversed {
            selection.range.end
        } else {
            selection.range.start
        };
        *selection = UTF16Selection {
            range: anchor.min(caret)..anchor.max(caret),
            reversed: caret < anchor,
        };
        self.reset_modal_caret();
        cx.notify();
    }
    pub(super) fn modal_replace_range(
        &mut self,
        range: std::ops::Range<usize>,
        text: &str,
        cx: &mut Context<Self>,
    ) {
        let field = self.explorer_modal_field;
        self.modal_replace_field_range(field, range, text, cx);
    }

    pub(super) fn modal_replace_field_range(
        &mut self,
        field: ModalField,
        range: std::ops::Range<usize>,
        text: &str,
        cx: &mut Context<Self>,
    ) {
        if field != ModalField::Name {
            let current = self.modal_field_text(field).to_owned();
            let start = range.start.min(current.encode_utf16().count());
            let end = range.end.min(current.encode_utf16().count());
            let (updated, caret) = replace_utf16_range(&current, start..end, text);
            *self.modal_field_text_mut(field) = updated;
            *self.modal_field_selection_mut(field) = UTF16Selection {
                range: caret..caret,
                reversed: false,
            };

            if field == ModalField::File {
                self.explorer_file_auto = false;
            }
            if matches!(field, ModalField::Extends | ModalField::Implements)
                && field == self.explorer_modal_field
            {
                self.refresh_modal_types();
            }
            self.reset_modal_caret();
            cx.notify();
            return;
        }
        self.reset_modal_caret();
        if self
            .explorer_operation
            .as_ref()
            .is_some_and(|operation| matches!(operation, ExplorerOperation::Rename(_)))
        {
            self.explorer_undo.push((
                self.explorer_input.clone(),
                UTF16Selection {
                    range: self.explorer_selection.range.clone(),
                    reversed: self.explorer_selection.reversed,
                },
            ));
        }
        let before = self.explorer_input.encode_utf16().count();
        let (updated, caret) = replace_utf16_range(&self.explorer_input, range.clone(), text);
        self.explorer_input = updated;
        if self.explorer_file_auto
            && self
                .explorer_operation
                .as_ref()
                .is_some_and(|op| matches!(op, ExplorerOperation::NewPhp { .. }))
        {
            self.explorer_file = format!("{}.php", self.explorer_input.trim_end_matches(".php"));
        }
        self.explorer_selection = UTF16Selection {
            range: caret..caret,
            reversed: false,
        };
        if debug_input_enabled() {
            tracing::info!(
                kind = if self
                    .explorer_operation
                    .as_ref()
                    .is_some_and(|op| matches!(op, ExplorerOperation::Rename(_)))
                {
                    "rename"
                } else {
                    "explorer"
                },
                range_start = range.start,
                range_end = range.end,
                inserted_len = text.encode_utf16().count(),
                "[MODAL REPLACE TEXT]"
            );
            if text.is_empty() {
                tracing::info!(
                    range_start = range.start,
                    range_end = range.end,
                    "[MODAL DELETE]"
                );
            }
            tracing::info!(
                value_len_before = before,
                value_len_after = self.explorer_input.encode_utf16().count(),
                changed = before != self.explorer_input.encode_utf16().count(),
                "[MODAL STATE]"
            );
        }
        cx.notify();
    }

    pub(super) fn reset_modal_caret(&mut self) {
        self.modal_caret_visible = true;
        let now = Instant::now();
        self.modal_caret_activity = now;
        self.modal_caret_toggle = now;
    }

    pub(super) fn modal_key_edit(
        &mut self,
        key: &str,
        modifiers: Modifiers,
        cx: &mut Context<Self>,
    ) -> bool {
        if matches!(key, "left" | "right" | "home" | "end") || (modifiers.control && key == "a") {
            self.modal_type_items.clear();
        }
        let field = self.explorer_modal_field;
        self.reset_modal_caret();
        let length = self.modal_field_text(field).encode_utf16().count();
        let start = self
            .modal_field_selection(field)
            .range
            .start
            .min(self.modal_field_selection(field).range.end);
        let end = self
            .modal_field_selection(field)
            .range
            .start
            .max(self.modal_field_selection(field).range.end);
        if modifiers.control
            && key == "z"
            && self
                .explorer_operation
                .as_ref()
                .is_some_and(|operation| matches!(operation, ExplorerOperation::Rename(_)))
        {
            if let Some((value, selection)) = self.explorer_undo.pop() {
                self.explorer_input = value;
                *self.modal_field_selection_mut(field) = selection;
                if debug_input_enabled() {
                    tracing::info!(
                        selection = ?self.modal_field_selection(field).range,
                        "[RENAME UNDO]"
                    );
                }
                cx.notify();
            }
            return true;
        }
        if modifiers.control && key == "a" {
            *self.modal_field_selection_mut(field) = UTF16Selection {
                range: 0..length,
                reversed: false,
            };
            cx.notify();
            return true;
        }
        if modifiers.control && key == "v" {
            if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                self.modal_replace_range(start..end, &text, cx);
            }
            return true;
        }
        if modifiers.control && matches!(key, "c" | "x") {
            if start != end {
                let query = self.modal_field_text(field);
                let text = query
                    [utf16_to_byte_offset(query, start)..utf16_to_byte_offset(query, end)]
                    .to_owned();
                cx.write_to_clipboard(ClipboardItem::new_string(text));
                if key == "x" {
                    self.modal_replace_range(start..end, "", cx);
                }
            }
            return true;
        }
        if key == "backspace" {
            if start != end {
                self.modal_replace_range(start..end, "", cx);
            } else if start > 0 {
                let previous = crate::ui::input_line::adjacent_utf16(
                    self.modal_field_text(field),
                    start,
                    false,
                );
                self.modal_replace_range(previous..start, "", cx);
            }
            return true;
        }
        if key == "delete" {
            if start != end {
                self.modal_replace_range(start..end, "", cx);
            } else if end < length {
                let next =
                    crate::ui::input_line::adjacent_utf16(self.modal_field_text(field), end, true);
                self.modal_replace_range(end..next, "", cx);
            }
            return true;
        }
        if matches!(key, "left" | "right") {
            let next = if key == "left" {
                if start != end {
                    start
                } else {
                    crate::ui::input_line::adjacent_utf16(
                        self.modal_field_text(field),
                        start,
                        false,
                    )
                }
            } else {
                if start != end {
                    end
                } else {
                    crate::ui::input_line::adjacent_utf16(self.modal_field_text(field), end, true)
                }
            };
            *self.modal_field_selection_mut(field) = UTF16Selection {
                range: next..next,
                reversed: false,
            };
            cx.notify();
            return true;
        }
        false
    }

    pub(super) fn confirm_explorer_operation(&mut self, cx: &mut Context<Self>) {
        if self.explorer_fs_busy {
            self.status = "Another file operation is in progress".into();
            cx.notify();
            return;
        }
        let Some(operation) = self.explorer_operation.as_ref() else {
            return;
        };
        let name = self.explorer_input.trim().to_owned();
        let is_php = matches!(
            operation,
            ExplorerOperation::NewPhp { .. } | ExplorerOperation::NewPhpFile(_)
        );
        if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\']) {
            self.status = "Name must be a valid PHP identifier".into();
            cx.notify();
            return;
        }
        let invalid_php = match operation {
            ExplorerOperation::NewPhpFile(_) => !valid_php_identifier(php_file_stem(&name)),
            ExplorerOperation::NewPhp { .. } => {
                !valid_php_identifier(php_file_stem(&name))
                    || !valid_php_namespace(&self.explorer_namespace)
                    || self.explorer_file.trim().is_empty()
                    || self.explorer_file.contains(['/', '\\'])
            }
            _ => false,
        };
        if is_php && invalid_php {
            self.status = "Invalid PHP name or namespace".into();
            cx.notify();
            return;
        }
        let Some(operation) = self.explorer_operation.take() else {
            return;
        };
        self.pending_created_caret = None;
        self.modal_focus_pending = false;
        self.explorer_input.clear();
        self.explorer_undo.clear();
        let Some(project) = self.project.clone() else {
            return;
        };
        let operation = match operation {
            ExplorerOperation::NewFile(directory) => (directory, name, None),
            ExplorerOperation::NewPhpFile(directory) => {
                let name = php_file_name(&name);
                (directory, name, None)
            }
            ExplorerOperation::NewPhp { directory, keyword } => {
                let name = php_file_name(&self.explorer_file);
                let symbol = php_file_stem(&name);
                let generated = crate::php_type_template::render_with_caret(
                    keyword,
                    symbol,
                    &self.explorer_namespace,
                    &self.explorer_extends,
                    &self.explorer_implements,
                );
                self.pending_created_caret = Some(generated.caret_offset);
                (directory, name, Some(generated.contents))
            }
            ExplorerOperation::NewDirectory(directory) => {
                self.explorer_fs_busy = true;
                cx.spawn(async move |this, cx| {
                    let result = gpui::background_executor()
                        .spawn(async move {
                            axiom_index::trace_path(
                                "explorer_create_directory",
                                "Create",
                                &directory,
                            );
                            project
                                .create_directory(&directory, &name)
                                .map(|_| ExplorerFsResult::Create(None))
                                .map_err(|e| e.to_string())
                        })
                        .await;
                    let _ = this.update(cx, |this, cx| {
                        this.explorer_fs_busy = false;
                        match result {
                            Ok(_) => {
                                this.refresh_explorer(cx);
                                this.status = "Directory created".into();
                            }
                            Err(error) => this.status = format!("Operation failed: {error}").into(),
                        }
                        cx.notify();
                    });
                })
                .detach();
                return;
            }
            ExplorerOperation::Rename(path) => {
                self.explorer_fs_busy = true;
                cx.spawn(async move |this, cx| {
                    let result = gpui::background_executor()
                        .spawn(async move {
                            axiom_index::trace_path("explorer_rename", "Rename", &path);
                            project
                                .rename(&path, &name)
                                .map(|new| ExplorerFsResult::Rename { old: path, new })
                                .map_err(|e| e.to_string())
                        })
                        .await;
                    let _ = this.update(cx, |this, cx| {
                        this.explorer_fs_busy = false;
                        match result {
                            Ok(ExplorerFsResult::Rename { old, new }) => {
                                let dirty_contents: HashMap<PathBuf, String> = this
                                    .tabs
                                    .iter()
                                    .filter(|tab| tab.path.starts_with(&old))
                                    .filter_map(|tab| {
                                        tab.editor.read(cx).is_dirty().then(|| {
                                            (
                                                tab.path.clone(),
                                                tab.editor.read(cx).document_content(),
                                            )
                                        })
                                    })
                                    .collect();
                                let semantic_changes = this
                                    .project_index
                                    .as_ref()
                                    .map(|index| {
                                        let paths = index
                                            .read()
                                            .map(|index| {
                                                index
                                                    .indexed_files()
                                                    .map(Path::to_path_buf)
                                                    .collect::<Vec<_>>()
                                            })
                                            .unwrap_or_default();
                                        paths
                                            .into_iter()
                                            .filter(|path| path.starts_with(&old))
                                            .filter_map(|path| {
                                                let relative = path.strip_prefix(&old).ok()?;
                                                let new_path = new.join(relative);
                                                let text =
                                                    dirty_contents.get(&path).cloned().or_else(
                                                        || fs::read_to_string(&new_path).ok(),
                                                    )?;
                                                Some((path, Some((new_path, text))))
                                            })
                                            .collect::<Vec<_>>()
                                    })
                                    .unwrap_or_default();
                                for tab in &mut this.tabs {
                                    if let Ok(relative) = tab.path.strip_prefix(&old) {
                                        tab.path = new.join(relative);
                                        tab.editor.update(cx, |editor, _| {
                                            editor.relocate_path(&old, &new)
                                        });
                                    }
                                }
                                this.schedule_semantic_fs_batch(semantic_changes, cx);
                                this.refresh_explorer(cx);
                                this.status = "Renamed".into();
                            }
                            _ => this.status = "Operation failed".into(),
                        }
                        cx.notify();
                    });
                })
                .detach();
                return;
            }
        };
        self.explorer_fs_busy = true;
        cx.spawn(async move |this, cx| {
            let result = gpui::background_executor()
                .spawn(async move {
                    axiom_index::trace_path("explorer_create_file", "Create", &operation.0);
                    let path = project
                        .create_file(&operation.0, &operation.1)
                        .map_err(|e| e.to_string())?;
                    if let Some(body) = operation.2 {
                        fs::write(&path, body).map_err(|e| e.to_string())?;
                    }
                    Ok::<_, String>(ExplorerFsResult::Create(Some(path)))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.explorer_fs_busy = false;
                match result {
                    Ok(ExplorerFsResult::Create(Some(path))) => {
                        this.refresh_explorer(cx);
                        let caret = this.pending_created_caret.take();
                        this.open_file_background_at(path, caret, cx);
                    }
                    Ok(_) => this.refresh_explorer(cx),
                    Err(error) => this.status = format!("Operation failed: {error}").into(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn request_delete(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if self.explorer_fs_busy {
            self.status = "Another file operation is in progress".into();
            cx.notify();
            return;
        }
        if debug_input_enabled() {
            tracing::info!(path = %path.display(), "[EXPLORER ACTION] action=delete");
        }
        self.explorer_context = None;
        if self
            .tabs
            .iter()
            .any(|tab| tab.path.starts_with(&path) && tab.editor.read(cx).is_dirty())
        {
            self.status = "Save or close modified files before deleting".into();
        } else {
            self.pending_delete_is_directory = fs::metadata(&path)
                .map(|metadata| metadata.is_dir())
                .unwrap_or(false);
            self.pending_delete = Some(path);
            self.delete_focus_pending = true;
            self.status = "Confirm deletion".into();
            if debug_input_enabled() {
                tracing::info!(confirmation_open = true, "[DELETE]");
            }
        }
        cx.notify();
    }

    pub(super) fn confirm_delete(&mut self, cx: &mut Context<Self>) {
        if self.explorer_fs_busy {
            self.status = "Another file operation is in progress".into();
            cx.notify();
            return;
        }
        let Some(path) = self.pending_delete.take() else {
            return;
        };
        self.delete_focus_pending = false;
        self.pending_delete_is_directory = false;
        let Some(project) = self.project.clone() else {
            self.status = "No project open".into();
            cx.notify();
            return;
        };
        self.explorer_fs_busy = true;
        cx.spawn(async move |this, cx| {
            let result = gpui::background_executor()
                .spawn(async move {
                    axiom_index::trace_path("explorer_delete", "Other", &path);
                    project
                        .delete(&path)
                        .map(|_| ExplorerFsResult::Delete(path))
                        .map_err(|e| e.to_string())
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.explorer_fs_busy = false;
                match result {
                    Ok(ExplorerFsResult::Delete(path)) => {
                        if debug_input_enabled() {
                            tracing::info!(success = true, "[DELETE RESULT]");
                        }
                        for tab in this.tabs.iter().filter(|tab| tab.path.starts_with(&path)) {
                            tab.editor.read(cx).close_lsp_document();
                        }
                        let semantic_changes = this
                            .project_index
                            .as_ref()
                            .map(|index| {
                                let paths = index
                                    .read()
                                    .map(|index| {
                                        index
                                            .indexed_files()
                                            .map(Path::to_path_buf)
                                            .collect::<Vec<_>>()
                                    })
                                    .unwrap_or_default();
                                paths
                                    .into_iter()
                                    .filter(|candidate| candidate.starts_with(&path))
                                    .map(|candidate| (candidate, None))
                                    .collect::<Vec<_>>()
                            })
                            .unwrap_or_default();
                        this.schedule_semantic_fs_batch(semantic_changes, cx);
                        this.tabs.retain(|tab| !tab.path.starts_with(&path));
                        this.active = if this.tabs.is_empty() {
                            None
                        } else {
                            Some(this.active.unwrap_or(0).min(this.tabs.len() - 1))
                        };
                        this.refresh_explorer(cx);
                        this.status = "Entry deleted".into();
                    }
                    Err(error) => {
                        if debug_input_enabled() {
                            tracing::info!(success = false, error = %error, "[DELETE RESULT]");
                        }
                        this.status = format!("Falha ao excluir: {error}").into();
                    }
                    _ => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn copy_path(&mut self, path: &Path, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(path.display().to_string()));
        self.explorer_context = None;
        self.status = "Path copied".into();
        cx.notify();
    }

    pub(super) fn render_explorer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme();
        let m = metrics();
        let workspace = cx.entity();
        let project_root = self
            .project
            .as_ref()
            .map(|project| project.root_path().to_path_buf());
        let root_name = self
            .project
            .as_ref()
            .map(Project::name)
            .unwrap_or("No project");
        let active_path = self
            .active
            .and_then(|index| self.tabs.get(index))
            .map(|tab| tab.path.clone());
        // Keep the tree viewport constrained by the sidebar while giving its
        // content a natural width. This enables GPUI's horizontal scrollbar
        // for deep trees and long names instead of shrinking/clipping rows.
        let tree_content_width = self
            .explorer
            .iter()
            .map(|item| {
                12.0 + item.depth as f32 * 16.0 + 28.0 + item.name.chars().count() as f32 * 8.0
            })
            .fold(180.0_f32, f32::max);
        let scroll_max: f32 = self.explorer_scroll.max_offset().height.into();
        let viewport_height: f32 = self.explorer_scroll.bounds().size.height.into();
        let content_height = (viewport_height + scroll_max).max(1.0);
        let thumb_height = (viewport_height * viewport_height / content_height)
            .max(24.0)
            .min(viewport_height.max(24.0));
        let offset_y: f32 = self.explorer_scroll.offset().y.into();
        let thumb_top = if scroll_max > 0.0 {
            (offset_y / scroll_max) * (viewport_height - thumb_height).max(0.0)
        } else {
            0.0
        };
        div()
            .id("explorer-sidebar")
            .w(self.project_panel_width)
            .min_w(px(180.))
            .h_full()
            .min_h(px(0.))
            .flex()
            .flex_col()
            .relative()
            .on_hover({
                let workspace = workspace.clone();
                move |hovered, _, cx| {
                    workspace.update(cx, |this, cx| {
                        this.explorer_scroll_hovered = *hovered;
                        cx.notify();
                    });
                }
            })
            .bg(t.sidebar_background)
            .border_r_1()
            .border_color(t.border_subtle)
            .child(
                div()
                    .h(m.panel_header_height)
                    .px_3()
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_size(m.ui_font_size)
                    .text_color(t.text_secondary)
                    .child("PROJECT")
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .when_some(project_root.clone(), |this, root| {
                                let new_file_workspace = workspace.clone();
                                let new_directory_workspace = workspace.clone();
                                let refresh_workspace = workspace.clone();
                                this.child(
                                    div()
                                        .id("explorer-new-file")
                                        .tooltip(|_, cx| tooltip("New", cx))
                                        .px_1()
                                        .w(m.toolbar_height)
                                        .h(m.toolbar_height)
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded(m.border_radius_small)
                                        .hover(move |s| s.bg(t.hover))
                                        .on_mouse_down(MouseButton::Left, |_, _, _| {
                                            if debug_input_enabled() {
                                                tracing::info!(
                                                    "[EXPLORER TOOLBAR MOUSE DOWN] button=new"
                                                );
                                            }
                                        })
                                        .on_click({
                                            let root = root.clone();
                                            move |_, window, cx| {
                                                new_file_workspace.update(cx, |this, cx| {
                                                    this.open_new_menu(root.clone(), cx);
                                                    let _ = window;
                                                })
                                            }
                                        })
                                        .child("+"),
                                )
                                .child(
                                    div()
                                        .id("explorer-new-directory")
                                        .tooltip(|_, cx| tooltip("New Directory", cx))
                                        .px_1()
                                        .w(m.toolbar_height)
                                        .h(m.toolbar_height)
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded(m.border_radius_small)
                                        .hover(move |s| s.bg(t.hover))
                                        .on_click({
                                            let root = root.clone();
                                            move |_, _, cx| {
                                                new_directory_workspace.update(cx, |this, cx| {
                                                    this.new_directory(root.clone(), cx)
                                                })
                                            }
                                        })
                                        .child("□+"),
                                )
                                .child(
                                    div()
                                        .id("explorer-refresh")
                                        .tooltip(|_, cx| tooltip("Refresh", cx))
                                        .px_1()
                                        .w(m.toolbar_height)
                                        .h(m.toolbar_height)
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded(m.border_radius_small)
                                        .hover(move |s| s.bg(t.hover))
                                        .on_click(move |_, _, cx| {
                                            refresh_workspace
                                                .update(cx, |this, cx| this.refresh_explorer(cx));
                                        })
                                        .child("↻"),
                                )
                            }),
                    ),
            )
            .child(
                div()
                    .h(m.toolbar_height)
                    .px_2()
                    .text_color(t.text_primary)
                    .when_some(project_root.clone(), |this, root| {
                        let workspace = workspace.clone();
                        this.on_mouse_down(MouseButton::Right, move |event, _, cx| {
                            workspace.update(cx, |this, cx| {
                                this.open_context_menu(
                                    root.clone(),
                                    EntryKind::Directory,
                                    event.position,
                                    cx,
                                );
                            });
                            cx.stop_propagation();
                        })
                    })
                    .child(format!("▾ {root_name}")),
            )
            .child(
                div()
                    .id("explorer-scroll")
                    .flex_1()
                    .min_h(px(0.))
                    .min_w(px(0.))
                    .overflow_scroll()
                    .scrollbar_width(px(8.))
                    .track_scroll(&self.explorer_scroll)
                    .on_mouse_move(cx.listener(Self::explorer_scroll_drag_move))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(Self::explorer_scroll_drag_end),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(Self::explorer_scroll_drag_end),
                    )
                    .child(div().w(px(tree_content_width)).flex().flex_col().children(
                        self.explorer.iter().enumerate().map(|(index, item)| {
                            let item = item.clone();
                            let workspace = workspace.clone();
                            let context_workspace = workspace.clone();
                            let context_item = item.clone();
                            let is_expanded = self.expanded.contains(&item.path);
                            let is_active = active_path.as_ref() == Some(&item.path);
                            let icon = file_icon(
                                &item.path,
                                item.kind == EntryKind::Directory,
                                is_expanded,
                            )
                            .glyph();
                            div()
                                .id(("explorer", index))
                                .w_full()
                                .h(px(24.))
                                .pl(px(12. + item.depth as f32 * 16.))
                                .flex()
                                .items_center()
                                .gap_2()
                                .text_size(m.ui_font_size)
                                .text_color(if is_active {
                                    t.text_primary
                                } else {
                                    t.text_secondary
                                })
                                .bg(if is_active {
                                    t.pressed
                                } else {
                                    t.sidebar_background
                                })
                                .hover(move |style| style.bg(t.hover))
                                .on_click(move |_, window, cx| {
                                    workspace.update(cx, |this, cx| {
                                        window.focus(&this.focus);
                                        if item.kind == EntryKind::Directory {
                                            this.selected_path = Some(item.path.clone());
                                            this.toggle_directory(index, cx);
                                        } else {
                                            this.selected_path = Some(item.path.clone());
                                            this.open_file(item.path.clone(), window, cx);
                                        }
                                    });
                                })
                                .on_mouse_down(MouseButton::Right, move |event, window, cx| {
                                    context_workspace.update(cx, |this, cx| {
                                        window.focus(&this.focus);
                                        this.open_context_menu(
                                            context_item.path.clone(),
                                            context_item.kind,
                                            event.position,
                                            cx,
                                        );
                                    });
                                    cx.stop_propagation();
                                })
                                .child(
                                    div()
                                        .w(m.icon_size)
                                        .text_color(if is_active { t.accent } else { t.text_muted })
                                        .child(icon),
                                )
                                .child(item.name)
                        }),
                    ))
                    .when(scroll_max > 0.0 && self.explorer_scroll_hovered, |this| {
                        this.child(
                            div()
                                .absolute()
                                .right(px(1.))
                                .top(px(1.))
                                .bottom(px(1.))
                                .w(px(6.))
                                .opacity(0.0)
                                .hover(|style| style.opacity(1.0))
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(Self::explorer_scroll_drag_start),
                                )
                                .child(
                                    div()
                                        .absolute()
                                        .left(px(0.))
                                        .right(px(0.))
                                        .top(px(thumb_top))
                                        .h(px(thumb_height))
                                        .rounded(px(3.))
                                        .bg(t.scrollbar_hover),
                                ),
                        )
                    }),
            )
            .child(
                div()
                    .id("explorer-resize-divider")
                    .absolute()
                    .right(px(-2.))
                    .top(px(0.))
                    .bottom(px(0.))
                    .w(px(4.))
                    .cursor(CursorStyle::ResizeLeftRight)
                    .hover(|style| style.bg(theme().accent))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(Self::project_panel_resize_start),
                    ),
            )
    }

    pub(super) fn explorer_scroll_drag_start(
        &mut self,
        event: &MouseDownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.explorer_scroll_dragging = true;
        self.explorer_scroll_drag_start_y = event.position.y.into();
        let offset: f32 = self.explorer_scroll.offset().y.into();
        self.explorer_scroll_drag_start_offset = -offset;
        cx.notify();
    }

    pub(super) fn explorer_scroll_drag_move(
        &mut self,
        event: &MouseMoveEvent,
        _: &mut Window,
        _: &mut Context<Self>,
    ) {
        if !self.explorer_scroll_dragging {
            return;
        }
        let viewport: f32 = self.explorer_scroll.bounds().size.height.into();
        let max: f32 = self.explorer_scroll.max_offset().height.into();
        let thumb = (viewport * viewport / (viewport + max).max(1.0)).max(24.0);
        let track = (viewport - thumb).max(1.0);
        let delta: f32 = event.position.y.into();
        let delta = delta - self.explorer_scroll_drag_start_y;
        let position =
            (self.explorer_scroll_drag_start_offset + delta * max / track).clamp(0.0, max);
        self.explorer_scroll
            .set_offset(Point::new(px(0.), px(-position)));
    }

    pub(super) fn explorer_scroll_drag_end(
        &mut self,
        _: &MouseUpEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.explorer_scroll_dragging = false;
        cx.notify();
    }

    pub(super) fn render_explorer_context(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = theme();
        let m = metrics();
        let workspace = cx.entity();
        let Some(context) = self.explorer_context.clone() else {
            return div();
        };
        let Some(project) = self.project.as_ref() else {
            return div();
        };
        let root = project.root_path().to_path_buf();
        let directory = Self::context_directory(&context.path, context.kind).to_path_buf();
        let is_root = context.path == root;
        let menu_width = px(300.);
        let submenu_width = px(250.);
        let opens_left = self.context_menu_position.x + menu_width + submenu_width
            > window.viewport_size().width;
        let menu_left = self
            .context_menu_position
            .x
            .max(px(0.))
            .min((window.viewport_size().width - menu_width).max(px(0.)));
        let menu_top = self
            .context_menu_position
            .y
            .max(px(0.))
            .min((window.viewport_size().height - px(360.)).max(px(0.)));
        div()
            .absolute()
            .left(menu_left)
            .top(menu_top)
            .w(menu_width)
            .p_1()
            .rounded(m.border_radius_medium)
            .border_1()
            .border_color(t.border)
            .flex()
            .flex_col()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .bg(t.menu_background)
            .text_color(t.text_primary)
            .when(context.kind == EntryKind::File, |this| {
                let open_workspace = workspace.clone();
                let path = context.path.clone();
                this.child(Self::explorer_menu_item("Open", move |window, cx| {
                    open_workspace.update(cx, |this, cx| this.open_file(path.clone(), window, cx));
                }))
                .child({
                    let workspace = workspace.clone();
                    Self::explorer_menu_item(
                        "Open Containing Folder in Terminal",
                        move |window, cx| {
                            workspace.update(cx, |this, cx| {
                                this.open_in_terminal(&OpenInTerminal, window, cx)
                            });
                        },
                    )
                })
            })
            .when(context.kind == EntryKind::Directory, |this| {
                let workspace = workspace.clone();
                this.child(Self::explorer_menu_item(
                    "Open in Terminal",
                    move |window, cx| {
                        workspace.update(cx, |this, cx| {
                            this.open_in_terminal(&OpenInTerminal, window, cx)
                        });
                    },
                ))
            })
            .child({
                let workspace = workspace.clone();
                let click_workspace = workspace.clone();
                let directory = directory.clone();
                let submenu = self.render_new_submenu(directory.clone(), opens_left, cx);
                div()
                    .relative()
                    .id("explorer-new-submenu-trigger")
                    .h(metrics().toolbar_height)
                    .px_2()
                    .flex()
                    .items_center()
                    .justify_between()
                    .rounded(metrics().border_radius_small)
                    .hover(move |style| style.bg(theme().hover))
                    .on_mouse_move(move |_, _, cx| {
                        workspace.update(cx, |this, cx| {
                            this.explorer_new_menu_open = true;
                            if debug_input_enabled() {
                                tracing::info!("[SUBMENU] open=New");
                            }
                            cx.notify();
                        });
                    })
                    .on_click({
                        move |_, _, cx| {
                            click_workspace.update(cx, |this, cx| this.open_context_submenu(cx));
                        }
                    })
                    .child("New")
                    .child("▶")
                    .when(self.explorer_new_menu_open, |this| this.child(submenu))
            })
            .when(!is_root, |this| {
                let rename_workspace = workspace.clone();
                let delete_workspace = workspace.clone();
                let rename_path = context.path.clone();
                let delete_path = context.path.clone();
                this.child(Self::explorer_menu_item("Rename  F2", move |window, cx| {
                    rename_workspace.update(cx, |this, cx| {
                        window.focus(&this.focus);
                        this.selected_path = Some(rename_path.clone());
                        this.execute_command("project.rename", window, cx);
                    });
                }))
                .child(Self::explorer_menu_item("Delete", move |_, cx| {
                    delete_workspace
                        .update(cx, |this, cx| this.request_delete(delete_path.clone(), cx));
                }))
            })
            .child({
                let workspace = workspace.clone();
                let path = context.path.clone();
                Self::explorer_menu_item("Copy Path", move |_, cx| {
                    workspace.update(cx, |this, cx| this.copy_path(&path, cx));
                })
            })
            .child({
                let workspace = workspace.clone();
                Self::explorer_menu_item("Refresh", move |_, cx| {
                    workspace.update(cx, |this, cx| this.refresh_explorer(cx));
                })
            })
    }

    pub(super) fn render_new_submenu(
        &self,
        directory: PathBuf,
        opens_left: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let workspace = cx.entity();
        let t = theme();
        let m = metrics();
        div()
            .absolute()
            .left(if opens_left { px(-296.) } else { px(296.) })
            .top(px(0.))
            .w(px(250.))
            .p_1()
            .flex()
            .flex_col()
            .gap_1()
            .bg(t.menu_background)
            .border_1()
            .border_color(t.border)
            .rounded(m.border_radius_medium)
            .shadow_lg()
            .on_mouse_move(|_, _, _| {})
            .child({
                let workspace = workspace.clone();
                let directory = directory.clone();
                Self::explorer_menu_item("File", move |window, cx| {
                    workspace.update(cx, |this, cx| {
                        this.begin_new_item(NewItemKind::File, directory.clone(), window, cx)
                    });
                })
            })
            .child({
                let workspace = workspace.clone();
                let directory = directory.clone();
                Self::explorer_menu_item("Directory", move |window, cx| {
                    workspace.update(cx, |this, cx| {
                        this.begin_new_item(NewItemKind::Directory, directory.clone(), window, cx)
                    });
                })
            })
            .child(div().h(px(1.)).mx_2().bg(t.border_subtle))
            .child({
                let workspace = workspace.clone();
                let directory = directory.clone();
                Self::explorer_menu_item("PHP File", move |window, cx| {
                    workspace.update(cx, |this, cx| {
                        this.begin_new_item(NewItemKind::PhpFile, directory.clone(), window, cx)
                    });
                })
            })
            .child({
                let workspace = workspace.clone();
                let directory = directory.clone();
                Self::explorer_menu_item("PHP Class", move |window, cx| {
                    workspace.update(cx, |this, cx| {
                        this.begin_new_item(NewItemKind::PhpClass, directory.clone(), window, cx)
                    });
                })
            })
            .child({
                let workspace = workspace.clone();
                let directory = directory.clone();
                Self::explorer_menu_item("PHP Interface", move |window, cx| {
                    workspace.update(cx, |this, cx| {
                        this.begin_new_item(
                            NewItemKind::PhpInterface,
                            directory.clone(),
                            window,
                            cx,
                        )
                    });
                })
            })
            .child(div())
    }
    pub(super) fn modal_text_field(
        &self,
        workspace: Entity<WorkspaceView>,
        label: &'static str,
        field: ModalField,
        value: String,
        window: &Window,
    ) -> impl IntoElement {
        let t = theme();
        let selected = self.modal_field_selection(field).range.clone();
        let focused = self.explorer_modal_field == field
            && self.modal_field_focus[field as usize].is_focused(window);
        let caret = if self.modal_field_selection(field).reversed {
            selected.start
        } else {
            selected.end
        }
        .min(value.encode_utf16().count());
        div()
            .relative()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(t.text_secondary)
                    .child(label),
            )
            .child(
                div()
                    .id(if field == ModalField::Name {
                        SharedString::from("explorer-operation-input")
                    } else {
                        SharedString::from(format!("php-input-{field:?}"))
                    })
                    .track_focus(&self.modal_field_focus[field as usize])
                    .debug_selector(move || format!("modal-field-{field:?}"))
                    .h(px(34.))
                    .w_full()
                    .px_2()
                    .flex()
                    .items_center()
                    .bg(t.panel_background)
                    .border_1()
                    .border_color(if focused { t.accent } else { t.border_subtle })
                    .cursor(CursorStyle::IBeam)
                    .on_mouse_down(MouseButton::Left, {
                        let workspace = workspace.clone();
                        move |event, window, cx| {
                            cx.stop_propagation();
                            workspace.update(cx, |this, cx| {
                                this.modal_pointer_down(field, event, window, cx);
                            });
                        }
                    })
                    .on_mouse_move({
                        let workspace = workspace.clone();
                        move |event, window, cx| {
                            workspace.update(cx, |this, cx| {
                                this.modal_pointer_move(field, event, window, cx)
                            });
                        }
                    })
                    .overflow_hidden()
                    .child(crate::ui::input_line::render(
                        self.modal_inputs[field as usize].clone(),
                        self.modal_field_focus[field as usize].clone(),
                        value,
                        if focused {
                            utf16_to_byte_offset(self.modal_field_text(field), selected.start)
                                ..utf16_to_byte_offset(self.modal_field_text(field), selected.end)
                        } else {
                            0..0
                        },
                        utf16_to_byte_offset(self.modal_field_text(field), caret),
                        self.modal_caret_for(field, window),
                        self.modal_field_geometry[field as usize].clone(),
                    )),
            )
            .when(focused && !self.modal_type_items.is_empty(), |this| {
                let anchor = self.modal_field_geometry[field as usize]
                    .anchor()
                    .unwrap_or_default();
                let geometry = modal_type_popup_geometry(
                    anchor,
                    window.viewport_size(),
                    self.modal_type_items.len(),
                    Some(px(630.)),
                );
                this.child(
                    gpui::deferred(
                        gpui::anchored()
                            .position_mode(gpui::AnchoredPositionMode::Window)
                            .position(geometry.origin)
                            .snap_to_window_with_margin(px(8.))
                            .child(self.modal_type_popup(workspace.clone(), geometry.size)),
                    )
                    .with_priority(2),
                )
            })
            .when(
                field == ModalField::Name
                    && matches!(
                        self.explorer_operation,
                        Some(ExplorerOperation::NewPhp { .. })
                    )
                    && !valid_php_identifier(php_file_stem(&self.explorer_input)),
                |this| {
                    this.child(
                        div()
                            .text_size(px(10.))
                            .text_color(t.error)
                            .child("Invalid PHP identifier"),
                    )
                },
            )
    }

    pub(super) fn modal_type_popup(
        &self,
        workspace: Entity<Self>,
        size: gpui::Size<Pixels>,
    ) -> impl IntoElement {
        let t = theme();
        div()
            .id("modal-type-completion")
            .debug_selector(|| "modal-type-completion".into())
            .w(size.width)
            .h(size.height)
            .overflow_y_scroll()
            .track_scroll(&self.modal_type_scroll)
            .bg(t.popup_background)
            .border_1()
            .border_color(t.border)
            .shadow_lg()
            .occlude()
            .children(
                self.modal_type_items
                    .iter()
                    .enumerate()
                    .map(|(index, item)| {
                        let workspace = workspace.clone();
                        div()
                            .id(("modal-type-candidate", index))
                            .debug_selector(move || format!("modal-type-candidate-{index}"))
                            .h(px(42.))
                            .px_2()
                            .flex()
                            .flex_col()
                            .justify_center()
                            .bg(if index == self.modal_type_selected {
                                t.hover
                            } else {
                                t.popup_background
                            })
                            .hover(move |style| style.bg(t.hover))
                            .cursor(CursorStyle::PointingHand)
                            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                                cx.stop_propagation();
                                workspace.update(cx, |this, cx| {
                                    this.accept_modal_type(index, cx);
                                    window.focus(
                                        &this.modal_field_focus[this.explorer_modal_field as usize],
                                    );
                                });
                            })
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(t.text_primary)
                                    .overflow_hidden()
                                    .child(item.label.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(t.text_secondary)
                                    .overflow_hidden()
                                    .child(item.detail.clone().unwrap_or_default()),
                            )
                    }),
            )
    }

    pub(super) fn render_explorer_operation(
        &self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let t = theme();
        let m = metrics();
        let workspace = cx.entity();
        let title = match self.explorer_operation {
            Some(ExplorerOperation::Rename(_)) => "Rename",
            Some(ExplorerOperation::NewDirectory(_)) => "New Directory",
            Some(ExplorerOperation::NewFile(_)) => "File",
            Some(ExplorerOperation::NewPhpFile(_)) => "PHP File",
            Some(ExplorerOperation::NewPhp { keyword, .. }) => match keyword {
                "class" => "Create PHP Class",
                "interface" => "Create PHP Interface",
                "trait" => "Create PHP Trait",
                "enum" => "Create PHP Enum",
                _ => "Create PHP Type",
            },
            None => "",
        };
        let php_valid = self
            .explorer_operation
            .as_ref()
            .is_some_and(|operation| match operation {
                ExplorerOperation::NewPhp { .. } => {
                    valid_php_identifier(php_file_stem(&self.explorer_input))
                        && valid_php_namespace(&self.explorer_namespace)
                }
                ExplorerOperation::NewPhpFile(_) => !self.explorer_input.trim().is_empty(),
                _ => true,
            });
        div()
            .absolute()
            .top(px(110.))
            .debug_selector(|| "php-type-modal".into())
            .left(px(320.))
            .w(px(520.))
            .max_h(px(680.))
            .p_4()
            .flex()
            .flex_col()
            .gap_2()
            .bg(t.popup_background)
            .border_1()
            .border_color(t.border)
            .rounded(m.border_radius_medium)
            .cursor(CursorStyle::Arrow)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .shadow_lg()
            .child(
                div()
                    .text_size(px(15.))
                    .text_color(t.text_primary)
                    .child(title),
            )
            .when(
                self.explorer_operation
                    .as_ref()
                    .is_some_and(|operation| matches!(operation, ExplorerOperation::NewPhp { .. })),
                |this| {
                    let directory = self.operation_directory();
                    let root = self.project.as_ref().map(|p| p.root_path());
                    this.child(
                        div()
                            .text_color(t.text_muted)
                            .text_size(px(11.))
                            .child("TYPE"),
                    )
                    .child(
                        div()
                            .id("php-type-segmented")
                            .w_full()
                            .h(px(34.))
                            .flex()
                            .items_center()
                            .rounded(m.border_radius_small)
                            .bg(t.panel_background)
                            .children([("class", "Class"), ("interface", "Interface"), ("trait", "Trait"), ("enum", "Enum")].into_iter().map(|(keyword, label)| {
                                let active = matches!(self.explorer_operation, Some(ExplorerOperation::NewPhp { keyword: current, .. }) if current == keyword);
                                let workspace = workspace.clone();
                                div().id(SharedString::from(format!("php-type-{keyword}"))).flex_1().h_full().flex().items_center().justify_center().px_2().rounded(m.border_radius_small)
                                    .bg(if active { t.inactive_selection } else { t.panel_background })
                                    .border_1().border_color(if active { t.accent } else { t.panel_background })
                                    .text_color(if active { t.text_primary } else { t.text_secondary })
                                    .cursor(CursorStyle::PointingHand).hover(move |style| if active { style } else { style.bg(t.hover) })
                                    .on_click(move |_, window, cx| workspace.update(cx, |this, cx| { this.select_php_type(keyword, cx); window.focus(&this.modal_field_focus[this.explorer_modal_field as usize]); }))
                                    .child(label)
                            }))
                    )
                    .child(
                        div()
                            .text_color(t.text_muted)
                            .text_size(px(11.))
                            .child("DIRECTORY"),
                    )
                    .child(
                        div()
                            .text_color(t.text_secondary)
                            .child(relative_directory_label(root.as_deref(), &directory)),
                    )
                },
            )
            .child(self.modal_text_field(workspace.clone(), "Name", ModalField::Name, self.explorer_input.clone(), _window))            .when(self.explorer_operation.as_ref().is_some_and(|op| matches!(op, ExplorerOperation::NewPhp { .. })), |this| {
                let workspace = workspace.clone();
                this.child(self.modal_text_field(workspace.clone(), "Namespace", ModalField::Namespace, self.explorer_namespace.clone(), _window))
                    .child(self.modal_text_field(workspace.clone(), "File", ModalField::File, self.explorer_file.clone(), _window))
                    .when(self.explorer_operation.as_ref().is_some_and(|op| matches!(op, ExplorerOperation::NewPhp { keyword: "class" | "interface", .. })), |form| {
                        form.child(self.modal_text_field(workspace.clone(), "Extends", ModalField::Extends, self.explorer_extends.clone(), _window))
                    })
                    .when(self.explorer_operation.as_ref().is_some_and(|op| matches!(op, ExplorerOperation::NewPhp { keyword: "class", .. })), |form| {
                        form.child(self.modal_text_field(workspace.clone(), "Implements", ModalField::Implements, self.explorer_implements.clone(), _window))
                    })
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .border_t_1()
                    .border_color(t.border_subtle)
                    .pt_3()
                    .mt_2()
                    .child(
                        div()
                            .id("explorer-operation-cancel")
                            .px_2()
                            .py_1()
                            .cursor(CursorStyle::PointingHand)
                            .on_click({
                                let workspace = workspace.clone();
                                move |_, _, cx| {
                                    workspace
                                        .update(cx, |this, cx| this.cancel_explorer_operation(cx));
                                }
                            })
                            .child("Cancel"),
                    )
                    .child(
                        div()
                            .id("explorer-operation-confirm")
                            .px_2()
                            .py_1()
                            .bg(if php_valid { t.accent } else { t.border_subtle })
                            .cursor(if php_valid {
                                CursorStyle::PointingHand
                            } else {
                                CursorStyle::Arrow
                            })
                            .text_color(if php_valid {
                                t.window_background
                            } else {
                                t.text_muted
                            })
                            .when(php_valid, |this| {
                                this.on_click(move |_, _, cx| {
                                    workspace
                                        .update(cx, |this, cx| this.confirm_explorer_operation(cx));
                                })
                            })
                            .child("Create"),
                    ),
            )
    }

    pub(super) fn operation_directory(&self) -> PathBuf {
        match self.explorer_operation.as_ref() {
            Some(ExplorerOperation::NewFile(directory))
            | Some(ExplorerOperation::NewPhpFile(directory))
            | Some(ExplorerOperation::NewPhp { directory, .. })
            | Some(ExplorerOperation::NewDirectory(directory)) => directory.clone(),
            Some(ExplorerOperation::Rename(path)) => path.parent().unwrap_or(path).to_path_buf(),
            None => PathBuf::new(),
        }
    }

    pub(super) fn explorer_menu_item(
        label: &'static str,
        handler: impl Fn(&mut Window, &mut App) + 'static,
    ) -> impl IntoElement {
        let t = theme();
        let m = metrics();
        div()
            .id(label)
            .h(m.toolbar_height)
            .px_2()
            .flex()
            .items_center()
            .rounded(m.border_radius_small)
            .hover(move |style| style.bg(t.hover))
            .on_mouse_down(MouseButton::Left, |_, _, cx| {
                if debug_input_enabled() {
                    tracing::info!("[SUBMENU ITEM MOUSE DOWN]");
                }
                cx.stop_propagation();
            })
            .on_click(move |_, window, cx| {
                if debug_input_enabled() {
                    tracing::info!(item = %label, "[CONTEXT MENU ACTION]");
                    tracing::info!(item = %label, "[SUBMENU ITEM CLICK]");
                }
                handler(window, cx);
                cx.stop_propagation();
            })
            .child(label)
    }
}
