use crate::{editor_view::EditorView, outline::OutlineItem, ui::theme};
use gpui::{
    Context, FocusHandle, KeyBinding, KeyDownEvent, Render, ScrollStrategy, Subscription,
    UniformListScrollHandle, WeakEntity, Window, actions, div, prelude::*, px, uniform_list,
};
use std::{ops::Range, path::PathBuf};

#[derive(Clone)]
pub(crate) struct Guard {
    pub session: u64,
    pub revision: u64,
    pub path: PathBuf,
}

actions!(outline, [Previous, Next, Accept, Cancel, Backspace, Delete]);

pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("up", Previous, Some("Outline")),
        KeyBinding::new("down", Next, Some("Outline")),
        KeyBinding::new("enter", Accept, Some("Outline")),
        KeyBinding::new("escape", Cancel, Some("Outline")),
        KeyBinding::new("backspace", Backspace, Some("Outline")),
        KeyBinding::new("delete", Delete, Some("Outline")),
    ]
}

fn filtered(items: &[OutlineItem], query: &str) -> Vec<usize> {
    let query = query.to_lowercase();
    let mut visible = vec![false; items.len()];
    for (i, item) in items.iter().enumerate() {
        if item.name.to_lowercase().starts_with(&query) {
            visible[i] = true;
            if let Some(parent) = item.parent {
                visible[parent] = true;
            }
        }
    }
    visible
        .into_iter()
        .enumerate()
        .filter_map(|(i, show)| show.then_some(i))
        .collect()
}

pub(crate) struct OutlinePopup {
    owner: WeakEntity<EditorView>,
    guard: Guard,
    items: Vec<OutlineItem>,
    visible: Vec<usize>,
    query: String,
    selected: usize,
    pub focus: FocusHandle,
    scroll: UniformListScrollHandle,
    subscriptions: Vec<Subscription>,
}

impl OutlinePopup {
    pub fn new(
        owner: WeakEntity<EditorView>,
        guard: Guard,
        items: Vec<OutlineItem>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            owner,
            guard,
            visible: (0..items.len()).collect(),
            items,
            query: String::new(),
            selected: 0,
            focus: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
            subscriptions: Vec::new(),
        }
    }
    pub fn watch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.subscriptions
            .push(cx.on_focus_out(&self.focus, window, |this, _, _, cx| {
                let _ = this
                    .owner
                    .update(cx, |editor, cx| editor.dismiss_outline(cx));
            }));
        if let Some(owner) = self.owner.upgrade() {
            self.subscriptions
                .push(cx.observe(&owner, |this, owner, cx| {
                    if !owner.read(cx).outline_is_current(&this.guard) {
                        let owner = this.owner.clone();
                        cx.defer(move |cx| {
                            let _ = owner.update(cx, |editor, cx| editor.dismiss_outline(cx));
                        });
                    }
                }));
        }
    }
    fn finish(&mut self, range: Option<Range<usize>>, window: &mut Window, cx: &mut Context<Self>) {
        let _ = self.owner.update(cx, |editor, cx| {
            editor.finish_outline(&self.guard, range, window, cx)
        });
    }
    fn accept(&mut self, _: &Accept, window: &mut Window, cx: &mut Context<Self>) {
        let range = self
            .visible
            .get(self.selected)
            .map(|i| self.items[*i].range.clone());
        self.finish(range, window, cx);
    }
    fn cancel(&mut self, _: &Cancel, window: &mut Window, cx: &mut Context<Self>) {
        self.finish(None, window, cx);
    }
    fn previous(&mut self, _: &Previous, _: &mut Window, cx: &mut Context<Self>) {
        self.selected = self.selected.saturating_sub(1);
        self.reveal(cx);
    }
    fn next(&mut self, _: &Next, _: &mut Window, cx: &mut Context<Self>) {
        self.selected = (self.selected + 1).min(self.visible.len().saturating_sub(1));
        self.reveal(cx);
    }
    fn reveal(&mut self, cx: &mut Context<Self>) {
        self.scroll
            .scroll_to_item(self.selected, ScrollStrategy::Top);
        cx.notify();
    }
    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.visible = filtered(&self.items, &self.query);
        let query = self.query.to_lowercase();
        self.selected = self
            .visible
            .iter()
            .position(|index| self.items[*index].name.to_lowercase().starts_with(&query))
            .unwrap_or(0);
        self.reveal(cx);
    }
    fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        self.query.pop();
        self.refresh(cx);
    }
    fn delete(&mut self, _: &Delete, _: &mut Window, _: &mut Context<Self>) {} // Filter caret is at the end.
}

impl Render for OutlinePopup {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme();
        let size = window.viewport_size();
        div()
            .id("file-structure-popup")
            .absolute()
            .top(px(8.))
            .left(px(8.))
            .w(px(440.).min((size.width - px(16.)).max(px(1.))))
            .bg(t.popup_background)
            .text_color(t.text_primary)
            .border_1()
            .border_color(t.border)
            .rounded_md()
            .shadow_lg()
            .occlude()
            .track_focus(&self.focus)
            .key_context("Outline")
            .on_action(cx.listener(Self::previous))
            .on_action(cx.listener(Self::next))
            .on_action(cx.listener(Self::accept))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if !event.keystroke.modifiers.control
                    && !event.keystroke.modifiers.platform
                    && !event.keystroke.modifiers.alt
                {
                    if let Some(text) = &event.keystroke.key_char {
                        this.query.push_str(text);
                        this.refresh(cx);
                    }
                }
                cx.stop_propagation();
            }))
            .child(div().p_2().child("File Structure"))
            .child(
                div()
                    .p_2()
                    .bg(t.editor_background)
                    .child(if self.query.is_empty() {
                        "Type to filter…".to_string()
                    } else {
                        format!("{}│", self.query)
                    }),
            )
            .when(self.visible.is_empty(), |el| {
                el.child(
                    div()
                        .p_2()
                        .text_color(t.text_secondary)
                        .child("No declarations"),
                )
            })
            .child(
                uniform_list(
                    "outline-list",
                    self.visible.len(),
                    cx.processor(|this, range: Range<usize>, _, cx| {
                        range
                            .map(|row| {
                                let item = &this.items[this.visible[row]];
                                let span = item.range.clone();
                                let selected = row == this.selected;
                                div()
                                    .id(("outline-row", row))
                                    .h(px(28.))
                                    .pl(px(8. + item.depth as f32 * 16.))
                                    .pr_2()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .debug_selector(move || format!("outline-row-{row}"))
                                    .bg(if row == this.selected {
                                        theme().selection
                                    } else {
                                        theme().popup_background
                                    })
                                    .hover(move |s| {
                                        s.bg(if selected {
                                            theme().selection
                                        } else {
                                            theme().hover
                                        })
                                    })
                                    .child(
                                        if matches!(item.kind, crate::outline::OutlineKind::Method)
                                        {
                                            format!("{}()", item.name)
                                        } else {
                                            item.name.clone()
                                        },
                                    )
                                    .child(
                                        div()
                                            .text_color(theme().text_muted)
                                            .text_xs()
                                            .child(format!("{:?}", item.kind)),
                                    )
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.finish(Some(span.clone()), window, cx);
                                    }))
                            })
                            .collect()
                    }),
                )
                .track_scroll(self.scroll.clone())
                .h(px((self.visible.len() as f32 * 28.).min(280.))
                    .min((size.height - px(100.)).max(px(0.)))),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outline::OutlineKind;
    #[test]
    fn filter_retains_owners_order_unicode_and_empty_results() {
        let items: Vec<_> = [
            ("A", None),
            ("save", Some(0)),
            ("B", None),
            ("saveAll", Some(2)),
            ("Árvore", None),
        ]
        .into_iter()
        .enumerate()
        .map(|(i, (name, parent))| OutlineItem {
            name: name.into(),
            parent,
            depth: usize::from(parent.is_some()),
            range: i..i + 1,
            kind: OutlineKind::Method,
        })
        .collect();
        assert_eq!(filtered(&items, ""), vec![0, 1, 2, 3, 4]);
        assert_eq!(filtered(&items, "SAV"), vec![0, 1, 2, 3]);
        assert_eq!(filtered(&items, "saveA"), vec![2, 3]);
        assert_eq!(filtered(&items, "ár"), vec![4]);
        assert!(filtered(&items, "missing").is_empty());
        assert!(filtered(&[], "").is_empty());
    }
}
