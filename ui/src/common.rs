//! Common UI components

use dioxus::prelude::*;

/// Props for inline editable text field
#[derive(Props, Clone, PartialEq)]
pub struct InlineEditProps {
    /// Current value to display
    pub value: Option<String>,
    /// Placeholder when value is empty
    #[props(default = "-".to_string())]
    pub placeholder: String,
    /// Called when save is clicked with the new value
    pub on_save: EventHandler<Option<String>>,
}

/// Inline editable text field component
/// Shows text with edit button, switches to input field when editing
#[component]
pub fn InlineEdit(props: InlineEditProps) -> Element {
    let mut editing = use_signal(|| false);
    let mut edit_value = use_signal(|| props.value.clone().unwrap_or_default());
    let mut saving = use_signal(|| false);

    // Clone values for closures
    let value_for_cancel = props.value.clone();
    let value_for_edit = props.value.clone();

    let handle_save = move |_| {
        let value = if edit_value.read().is_empty() {
            None
        } else {
            Some(edit_value.read().clone())
        };
        saving.set(true);
        props.on_save.call(value);
        editing.set(false);
        saving.set(false);
    };

    let handle_cancel = move |_| {
        edit_value.set(value_for_cancel.clone().unwrap_or_default());
        editing.set(false);
    };

    let handle_edit = move |_| {
        edit_value.set(value_for_edit.clone().unwrap_or_default());
        editing.set(true);
    };

    let display_value = props.value.clone();
    let placeholder = props.placeholder.clone();

    rsx! {
        if *editing.read() {
            div { class: "flex gap-2",
                input {
                    class: "flex-1 px-2 py-1 text-sm border border-gray-200 rounded focus:outline-none focus:ring-2 focus:ring-blue-500",
                    r#type: "text",
                    value: "{edit_value}",
                    oninput: move |e| edit_value.set(e.value())
                }
                button {
                    class: "px-2 py-1 text-xs bg-blue-600 text-white rounded hover:bg-blue-700 disabled:opacity-50",
                    disabled: *saving.read(),
                    onclick: handle_save,
                    "Save"
                }
                button {
                    class: "px-2 py-1 text-xs bg-gray-100 text-gray-700 rounded hover:bg-gray-200",
                    onclick: handle_cancel,
                    "Cancel"
                }
            }
        } else {
            div { class: "flex items-center gap-2 group",
                span { class: "text-gray-600 text-sm",
                    if let Some(val) = &display_value {
                        "{val}"
                    } else {
                        "{placeholder}"
                    }
                }
                button {
                    class: "px-2 py-1 text-xs bg-gray-100 text-gray-700 rounded hover:bg-gray-200 opacity-0 group-hover:opacity-100 transition-opacity",
                    onclick: handle_edit,
                    "Edit"
                }
            }
        }
    }
}
