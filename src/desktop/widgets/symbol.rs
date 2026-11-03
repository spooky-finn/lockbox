//! Small, bundled line icons shared by the interface.

use dioxus::prelude::*;

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Icon {
    Cloud,
    Lock,
    Key,
    Folder,
    FolderOpen,
    ChevronRight,
    FolderPlus,
    FolderX,
    Plus,
    Search,
    Settings,
    Copy,
    Eye,
    EyeOff,
    Edit,
    Trash,
    ArrowLeft,
    ArrowRight,
    Fingerprint,
    Save,
    Close,
    Sparkles,
    Check,
    Image,
    Reset,
    Clock,
}

#[component]
pub(crate) fn Symbol(icon: Icon) -> Element {
    let path = match icon {
        Icon::Cloud => "M6 18a5 5 0 0 1-1-9.9 7 7 0 0 1 13.5-1A5.5 5.5 0 0 1 18.5 18H6Z",
        Icon::Lock => {
            "M7 10V7a5 5 0 0 1 10 0v3M6 10h12a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2v-8a2 2 0 0 1 2-2Zm6 5v3"
        }
        Icon::Key => "M16 3a5 5 0 1 1-3 9l-3 3H7v3H4v3H2v-3l9-10a5 5 0 0 1 5-5Zm1 3h.01",
        Icon::Folder => {
            "M3 7V5a2 2 0 0 1 2-2h5l2 3h7a2 2 0 0 1 2 2v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7Zm0 1h18"
        }
        Icon::FolderOpen => {
            "M3 10V5a2 2 0 0 1 2-2h5l2 3h7a2 2 0 0 1 2 2v2M3 10h18l-3 11H3a1 1 0 0 1-1-1V10Z"
        }
        Icon::ChevronRight => "m9 5 7 7-7 7",
        Icon::FolderPlus => {
            "M3 7V5a2 2 0 0 1 2-2h5l2 3h7a2 2 0 0 1 2 2v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7Zm5 7h8m-4-4v8"
        }
        Icon::FolderX => {
            "M3 7V5a2 2 0 0 1 2-2h5l2 3h7a2 2 0 0 1 2 2v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7Zm6 4 6 6m0-6-6 6"
        }
        Icon::Plus => "M12 5v14M5 12h14",
        Icon::Search => "m21 21-5-5M18 10a8 8 0 1 1-16 0 8 8 0 0 1 16 0Z",
        Icon::Settings => "M4 7h10m4 0h2M4 17h2m4 0h10M14 4v6M10 14v6",
        Icon::Copy => {
            "M9 8V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2h-4M4 8h10a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V10a2 2 0 0 1 2-2Z"
        }
        Icon::Eye => {
            "M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12Zm13 0a3 3 0 1 1-6 0 3 3 0 0 1 6 0Z"
        }
        Icon::EyeOff => {
            "m3 3 18 18M10.6 10.6a2 2 0 0 0 2.8 2.8M9.9 5.2C17 3.8 22 12 22 12a19 19 0 0 1-3.2 4.1M6 6C3.4 8.4 2 12 2 12s3.5 7 10 7a10 10 0 0 0 4-.9"
        }
        Icon::Edit => "m15 5 4 4M4 20l4-1L21 6a2.8 2.8 0 0 0-4-4L4 15l-1 6 5-2",
        Icon::Trash => "M3 6h18M9 6V3h6v3M5 6l1 15h12l1-15M10 10v7m4-7v7",
        Icon::ArrowLeft => "m12 5-7 7 7 7M5 12h15",
        Icon::ArrowRight => "m12 5 7 7-7 7M19 12H5",
        Icon::Fingerprint => {
            "M4 11a8 8 0 0 1 16 0m-13 1v-1a5 5 0 0 1 10 0v3c0 3-.8 5-2 7M10 18c.8-2 1-4 1-7a1 1 0 0 1 2 0v3c0 3-.5 5-1.5 7M4 15c0 2-.3 3-1 4m4-4c0 3-.4 4.5-1.3 6M20 15c0 2-.2 3.5-.7 5"
        }
        Icon::Save => {
            "M19 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h12l4 4v12a2 2 0 0 1-2 2ZM17 21v-8H7v8M7 3v5h8V3"
        }
        Icon::Close => "m6 6 12 12M6 18 18 6",
        Icon::Sparkles => {
            "m12 3 2.5 6.5L21 12l-6.5 2.5L12 21l-2.5-6.5L3 12l6.5-2.5L12 3Zm7-1v4m-2-2h4"
        }
        Icon::Check => "m5 12 4 4L19 6",
        Icon::Image => {
            "M5 3h14a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2Zm-2 14 5-5 4 4 4-4 5 5M8 7h.01"
        }
        Icon::Reset => "M3 10a9 9 0 1 1 2 8M3 4v6h6",
        Icon::Clock => "M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0Zm-9-5v5l3 2",
    };
    rsx! {
        svg {
            class: "symbol", view_box: "0 0 24 24", fill: "none",
            stroke: "currentColor", stroke_width: "1.7", stroke_linecap: "round", stroke_linejoin: "round",
            "aria-hidden": "true", "focusable": "false",
            path { d: path }
        }
    }
}
