//! Everything that can be placed on a screen, the device sizes and the icon names.

pub struct Kind {
    pub id: &'static str,
    pub name: &'static str,
    pub cat: &'static str,
    pub w: f32,
    pub h: f32,
    pub text: &'static str,
    pub value: f32,
    /// How the text is read, shown in the inspector and to AI assistants.
    pub help: &'static str,
}

pub const CATS: &[&str] = &["Basic", "Text", "Inputs", "Navigation", "Content", "Mobile", "Charts & media", "Feedback", "Devices"];

const LIST: &str = "One item per line. Start a line with * for the active item, - for disabled, :icon for an icon (e.g. :home Home).";
const ITEMS: &str = "Items separated by commas. Start an item with * for the active one, - for disabled, :icon for an icon.";

macro_rules! k {
    ($id:expr, $name:expr, $cat:expr, $w:expr, $h:expr, $text:expr) => {
        Kind { id: $id, name: $name, cat: $cat, w: $w, h: $h, text: $text, value: 0.0, help: "" }
    };
    ($id:expr, $name:expr, $cat:expr, $w:expr, $h:expr, $text:expr, $value:expr, $help:expr) => {
        Kind { id: $id, name: $name, cat: $cat, w: $w, h: $h, text: $text, value: $value, help: $help }
    };
}

pub static KINDS: &[Kind] = &[
    // Basic
    k!("rect", "Box", "Basic", 200.0, 120.0, "", 0.0, "Optional text in the middle."),
    k!("circle", "Circle", "Basic", 100.0, 100.0, ""),
    k!("hline", "Horizontal line", "Basic", 200.0, 12.0, ""),
    k!("vline", "Vertical line", "Basic", 12.0, 200.0, ""),
    k!("arrow", "Arrow", "Basic", 160.0, 40.0, "", 0.0, "Optional label above the arrow. Set Value to 1 for a two-way arrow."),
    k!("bracket", "Curly brace", "Basic", 30.0, 160.0, ""),
    k!("image", "Image", "Basic", 200.0, 140.0, "", 0.0, "Placeholder for a picture. Optional caption."),
    k!("icon", "Icon", "Basic", 32.0, 32.0, "star", 0.0, "The icon's name, e.g. home, search, user, heart, star, settings."),
    k!("container", "Panel", "Basic", 260.0, 180.0, "Panel title", 0.0, "First line is the title (leave empty for no title bar)."),
    k!("card", "Card", "Basic", 260.0, 280.0, "Card title\nShort description goes here.", 0.0, "First line is the title, the rest is the body. Picture on top."),
    k!("sticky", "Sticky note", "Basic", 160.0, 140.0, "Remember to check this with the team."),
    k!("callout", "Numbered callout", "Basic", 30.0, 30.0, "1", 0.0, "A number or letter to point at things and refer to in notes."),
    k!("scribble", "Scribble", "Basic", 160.0, 60.0, ""),
    // Text
    k!("heading", "Title", "Text", 260.0, 40.0, "A big title"),
    k!("subheading", "Subtitle", "Text", 220.0, 30.0, "A subtitle"),
    k!("label", "Label", "Text", 120.0, 24.0, "Some text"),
    k!("paragraph", "Paragraph", "Text", 300.0, 110.0, "A short paragraph of sample text that wraps onto several lines to show where the copy goes on this screen.", 0.0, "Wraps to the box. Blank line = new paragraph."),
    k!("link", "Link", "Text", 90.0, 24.0, "A link"),
    k!("link_bar", "Link bar", "Text", 300.0, 24.0, "Home,*About,Blog,Contact", 0.0, ITEMS),
    k!("code", "Code block", "Text", 300.0, 120.0, "fn main() {\n    println!(\"hello\");\n}", 0.0, "Shown in a monospace box."),
    k!("quote", "Quote", "Text", 300.0, 80.0, "Something wise someone once said.\n- A person"),
    k!("bullets", "Bulleted list", "Text", 240.0, 100.0, "First point\nSecond point\nThird point", 0.0, "One bullet per line. Start with 1. for numbers."),
    // Inputs
    k!("button", "Button", "Inputs", 120.0, 40.0, "Button", 0.0, "Label. Start with :icon to add an icon. Selected state = pressed."),
    k!("button_primary", "Primary button", "Inputs", 140.0, 44.0, "Continue"),
    k!("icon_button", "Icon button", "Inputs", 40.0, 40.0, "plus", 0.0, "The icon's name."),
    k!("text_input", "Text field", "Inputs", 220.0, 40.0, "Placeholder", 0.0, "Placeholder or typed text. Focused state shows a cursor."),
    k!("text_area", "Text area", "Inputs", 260.0, 110.0, "Write something…"),
    k!("search", "Search field", "Inputs", 240.0, 40.0, "Search"),
    k!("password", "Password field", "Inputs", 220.0, 40.0, "Password"),
    k!("dropdown", "Dropdown", "Inputs", 180.0, 40.0, "Choose one", 0.0, "The chosen value. Focused state shows the open list (one option per line after the first)."),
    k!("checkbox", "Checkbox", "Inputs", 160.0, 24.0, "Remember me", 0.0, "Selected state = checked."),
    k!("checkbox_group", "Checkbox group", "Inputs", 180.0, 90.0, "[x] Option one\n[ ] Option two\n[ ] Option three", 0.0, "One per line: [x] checked, [ ] not, [-] disabled."),
    k!("radio", "Radio button", "Inputs", 160.0, 24.0, "Option"),
    k!("radio_group", "Radio group", "Inputs", 180.0, 90.0, "(o) Small\n( ) Medium\n( ) Large", 0.0, "One per line: (o) chosen, ( ) not."),
    k!("toggle", "Switch", "Inputs", 200.0, 32.0, "Notifications", 0.0, "Label on the left. Selected state = on."),
    k!("slider", "Slider", "Inputs", 200.0, 24.0, "", 50.0, "Value is the position, 0 to 100."),
    k!("stepper", "Number stepper", "Inputs", 130.0, 40.0, "1"),
    k!("date_field", "Date field", "Inputs", 180.0, 40.0, "03/11/2026"),
    k!("date_picker", "Calendar", "Inputs", 260.0, 270.0, "November 2026", 3.0, "Month title. Value is the chosen day."),
    k!("time_picker", "Time picker", "Inputs", 200.0, 140.0, "09:41"),
    k!("otp", "Code input", "Inputs", 260.0, 52.0, "4,2,_,_,_,_", 0.0, "One box per item, _ for empty."),
    k!("chips", "Chips", "Inputs", 300.0, 36.0, "*All,Music,Podcasts", 0.0, ITEMS),
    k!("segmented", "Segmented control", "Inputs", 260.0, 36.0, "*Day,Week,Month", 0.0, ITEMS),
    k!("rating", "Star rating", "Inputs", 130.0, 24.0, "", 3.0, "Value is the number of stars (0-5)."),
    k!("color_picker", "Colour swatches", "Inputs", 200.0, 40.0, "#e05a47,#f2b134,#4fb06d,#3d7dd8,#8e5bd0", 0.0, "Colours separated by commas."),
    k!("file_drop", "File upload", "Inputs", 300.0, 140.0, "Drop files here or browse"),
    k!("captcha", "I'm not a robot", "Inputs", 280.0, 70.0, "I'm not a robot"),
    // Navigation
    k!("tabs", "Tabs", "Navigation", 360.0, 200.0, "*Home,Profile,Settings", 0.0, ITEMS),
    k!("breadcrumbs", "Breadcrumbs", "Navigation", 300.0, 24.0, "Home > Products > Detail", 0.0, "Items separated by >."),
    k!("pagination", "Pagination", "Navigation", 280.0, 36.0, "<,1,*2,3,…,9,>", 0.0, ITEMS),
    k!("menu", "Menu", "Navigation", 180.0, 170.0, "Open\n*Save\n-Save as…\n---\nQuit", 0.0, "One item per line, --- for a divider. * active, - disabled, :icon for icons. Add ⌘X after a tab for a shortcut."),
    k!("menubar", "Menu bar", "Navigation", 400.0, 30.0, "File,Edit,View,Help", 0.0, ITEMS),
    k!("navbar", "Website header", "Navigation", 800.0, 64.0, "Brand\n*Home,Products,Pricing,About\nSign in", 0.0, "Line 1: logo text. Line 2: links (commas). Line 3: button."),
    k!("sidebar", "Side navigation", "Navigation", 220.0, 400.0, "*:home Home\n:search Explore\n:bell Notifications\n:settings Settings", 0.0, LIST),
    k!("vertical_tabs", "Vertical tabs", "Navigation", 300.0, 200.0, "*General\nPrivacy\nAdvanced", 0.0, LIST),
    k!("steps", "Progress steps", "Navigation", 420.0, 60.0, "Cart,*Shipping,Payment,Done", 0.0, ITEMS),
    k!("accordion", "Accordion", "Navigation", 280.0, 220.0, "*What is it?\nA short answer that explains it.\nHow much does it cost?\nCan I cancel?", 0.0, "One heading per line; * opens a heading and the next line is its content."),
    k!("tree", "Tree", "Navigation", 220.0, 180.0, "v Project\n  v src\n    main.rs\n    *app.rs\n  > docs\n  README.md", 0.0, "Indent with spaces. v open folder, > closed, * selected."),
    k!("command_palette", "Command palette", "Navigation", 420.0, 260.0, "Type a command…\n*:file New file\t⌘N\n:search Find\t⌘F\n:settings Settings\t⌘,", 0.0, "First line is the search text, then one result per line."),
    // Content
    k!("list", "List", "Content", 240.0, 180.0, "*First item\nSecond item\nThird item\nFourth item", 0.0, LIST),
    k!("table", "Table", "Content", 400.0, 160.0, "Name,Role,Status\nAlex,Designer,Active\n*Sam,Engineer,Away\nKim,Manager,Active", 0.0, "Rows on lines, cells separated by commas. First row is the header, * selects a row."),
    k!("avatar", "Avatar", "Content", 48.0, 48.0, "AB"),
    k!("avatar_group", "Avatar group", "Content", 140.0, 40.0, "AB,CD,EF,+4"),
    k!("badge", "Badge", "Content", 40.0, 22.0, "3"),
    k!("tag", "Tag", "Content", 80.0, 26.0, "New"),
    k!("stat", "Stat tile", "Content", 220.0, 110.0, "Revenue\n$24.3k\n+12%", 0.0, "Line 1 label, line 2 big number, line 3 change."),
    k!("chat", "Chat bubble", "Content", 240.0, 60.0, "Hey! Are we still on for today?", 0.0, "Selected state = sent by me (right side)."),
    k!("comment_thread", "Comment", "Content", 280.0, 90.0, "Dana · 2h\nCan we make this button bigger?"),
    k!("pricing", "Pricing card", "Content", 240.0, 320.0, "Pro\n$12/mo\nUnlimited projects\nAI assistant\nPriority support\nChoose Pro"),
    k!("timeline", "Timeline", "Content", 260.0, 200.0, "Order placed\n*Shipped\nOut for delivery\nDelivered", 0.0, LIST),
    k!("kanban", "Board column", "Content", 220.0, 300.0, "To do\nWrite copy\nPick colours\nUser test", 0.0, "First line is the column title, then one card per line."),
    k!("empty_state", "Empty state", "Content", 300.0, 200.0, "Nothing here yet\nCreate your first project to get started.\nCreate project"),
    k!("skeleton", "Loading placeholder", "Content", 280.0, 100.0, ""),
    k!("carousel", "Carousel", "Content", 340.0, 200.0, "", 1.0, "Value is the current slide (dots)."),
    k!("qr", "QR code", "Content", 110.0, 110.0, ""),
    k!("cookie_banner", "Cookie banner", "Content", 600.0, 70.0, "We use cookies to improve your experience.\nAccept,Settings"),
    // Mobile
    k!("status_bar", "Status bar", "Mobile", 393.0, 54.0, "9:41"),
    k!("app_bar", "Top app bar", "Mobile", 393.0, 56.0, ":chevron_left Title :more_v", 0.0, "Start/end with :icon for buttons, the rest is the title."),
    k!("bottom_nav", "Bottom navigation", "Mobile", 393.0, 74.0, "*:home Home,:search Search,:heart Saved,:user Profile", 0.0, ITEMS),
    k!("fab", "Floating button", "Mobile", 56.0, 56.0, "plus", 0.0, "The icon's name."),
    k!("bottom_sheet", "Bottom sheet", "Mobile", 393.0, 300.0, "Share to\n:mail Email\n:chat Messages\n:link Copy link"),
    k!("action_sheet", "Action sheet", "Mobile", 360.0, 220.0, "Delete this photo?\n-Delete\nCancel", 0.0, "Title then actions; - marks a destructive action."),
    k!("notification", "Push notification", "Mobile", 360.0, 76.0, "Messages · now\nDana: See you at 6!"),
    k!("keyboard", "Keyboard", "Mobile", 393.0, 260.0, ""),
    k!("search_bar", "Search bar", "Mobile", 360.0, 44.0, "Search"),
    k!("swipe_row", "Swipe row", "Mobile", 360.0, 64.0, "Swipe me\nArchive,Delete"),
    k!("home_widget", "Home widget", "Mobile", 170.0, 170.0, "Weather\n21°\nSunny"),
    k!("dynamic_island", "Live activity pill", "Mobile", 220.0, 40.0, "Timer 04:21"),
    k!("page_dots", "Page dots", "Mobile", 80.0, 16.0, "", 1.0, "Value is the current page."),
    // Charts & media
    k!("chart_bar", "Bar chart", "Charts & media", 260.0, 170.0, "40,70,55,90,65", 0.0, "Numbers separated by commas."),
    k!("chart_line", "Line chart", "Charts & media", 260.0, 170.0, "30,45,40,60,55,75", 0.0, "Numbers separated by commas."),
    k!("chart_pie", "Pie chart", "Charts & media", 200.0, 170.0, "50,30,20", 0.0, "Numbers separated by commas."),
    k!("chart_donut", "Donut / progress ring", "Charts & media", 90.0, 90.0, "", 65.0, "Value is the percent."),
    k!("sparkline", "Sparkline", "Charts & media", 120.0, 32.0, "3,5,4,7,6,9,8"),
    k!("video", "Video player", "Charts & media", 320.0, 200.0, "", 30.0, "Value is how far it has played."),
    k!("audio", "Audio player", "Charts & media", 300.0, 60.0, "Episode 12", 40.0, "Value is how far it has played."),
    k!("map", "Map", "Charts & media", 280.0, 190.0, ""),
    k!("webcam", "Video call tile", "Charts & media", 220.0, 150.0, "Sam"),
    // Feedback
    k!("alert", "Alert banner", "Feedback", 360.0, 52.0, "Your changes were saved.", 0.0, "States: normal info, selected success, focused warning, disabled error."),
    k!("toast", "Toast", "Feedback", 320.0, 48.0, "Message sent\tUndo"),
    k!("tooltip", "Tooltip", "Feedback", 170.0, 44.0, "Helpful hint"),
    k!("dialog", "Dialog", "Feedback", 360.0, 200.0, "Delete project?\nThis can't be undone.\nCancel,*Delete", 0.0, "Line 1 title, middle lines body, last line buttons (commas, * main)."),
    k!("progress", "Progress bar", "Feedback", 220.0, 14.0, "", 60.0, "Value is the percent."),
    k!("spinner", "Spinner", "Feedback", 40.0, 40.0, ""),
    k!("ai_prompt", "AI prompt box", "Feedback", 420.0, 96.0, "Ask anything…", 0.0, "A modern prompt field with send button."),
    k!("ai_answer", "AI answer", "Feedback", 420.0, 120.0, "Here is a summary of your week: three meetings, two deadlines and one free afternoon."),
    // Devices (frames you can drop on a large canvas)
    k!("frame_phone", "Phone", "Devices", 260.0, 540.0, ""),
    k!("frame_android", "Android phone", "Devices", 260.0, 560.0, ""),
    k!("frame_classic", "Phone (home button)", "Devices", 240.0, 480.0, ""),
    k!("frame_foldable", "Foldable", "Devices", 460.0, 540.0, ""),
    k!("frame_flip", "Flip phone", "Devices", 240.0, 520.0, ""),
    k!("frame_tablet", "Tablet", "Devices", 480.0, 640.0, ""),
    k!("frame_laptop", "Laptop", "Devices", 640.0, 420.0, ""),
    k!("frame_desktop", "Monitor", "Devices", 640.0, 470.0, ""),
    k!("frame_browser", "Browser", "Devices", 640.0, 420.0, "https://example.com"),
    k!("frame_window", "App window", "Devices", 520.0, 360.0, "Window title"),
    k!("frame_watch", "Watch", "Devices", 200.0, 260.0, ""),
    k!("frame_watch_round", "Round watch", "Devices", 220.0, 260.0, ""),
    k!("frame_tv", "TV", "Devices", 640.0, 400.0, ""),
    k!("frame_car", "Car display", "Devices", 640.0, 280.0, ""),
    k!("frame_spatial", "Spatial (VR) window", "Devices", 560.0, 380.0, ""),
    k!("component", "Component", "Devices", 300.0, 60.0, "", 0.0, "Shows a component screen. Edit that screen and every copy updates."),
];

pub fn kind(id: &str) -> Option<&'static Kind> {
    KINDS.iter().find(|k| k.id == id).or_else(|| match id {
        "textinput" | "input" => kind("text_input"),
        "combo" | "combobox" | "select" => kind("dropdown"),
        "switch" => kind("toggle"),
        "title" | "h1" => kind("heading"),
        "text" => kind("label"),
        "rectangle" | "box" => kind("rect"),
        "line" => kind("hline"),
        "textarea" => kind("text_area"),
        "modal" => kind("dialog"),
        "snackbar" => kind("toast"),
        "banner" => kind("alert"),
        _ => None,
    })
}

pub struct Device {
    pub id: &'static str,
    pub name: &'static str,
    pub w: f32,
    pub h: f32,
    /// The frame drawn around the screen (see draw::frame).
    pub frame: &'static str,
}

pub static DEVICES: &[Device] = &[
    Device { id: "phone_island", name: "Phone, modern (393×852)", w: 393.0, h: 852.0, frame: "phone" },
    Device { id: "phone_max", name: "Phone, large (430×932)", w: 430.0, h: 932.0, frame: "phone" },
    Device { id: "android", name: "Android phone (412×915)", w: 412.0, h: 915.0, frame: "android" },
    Device { id: "phone_classic", name: "Phone, home button (375×667)", w: 375.0, h: 667.0, frame: "classic" },
    Device { id: "foldable_open", name: "Foldable, open (673×841)", w: 673.0, h: 841.0, frame: "foldable" },
    Device { id: "foldable_cover", name: "Foldable, cover (360×860)", w: 360.0, h: 860.0, frame: "android" },
    Device { id: "flip_cover", name: "Flip phone, cover (260×260)", w: 260.0, h: 260.0, frame: "flip" },
    Device { id: "tablet", name: "Tablet portrait (834×1194)", w: 834.0, h: 1194.0, frame: "tablet" },
    Device { id: "tablet_land", name: "Tablet landscape (1194×834)", w: 1194.0, h: 834.0, frame: "tablet" },
    Device { id: "laptop", name: "Laptop (1440×900)", w: 1440.0, h: 900.0, frame: "laptop" },
    Device { id: "desktop", name: "Desktop monitor (1920×1080)", w: 1920.0, h: 1080.0, frame: "desktop" },
    Device { id: "browser", name: "Web page in browser (1280×800)", w: 1280.0, h: 800.0, frame: "browser" },
    Device { id: "browser_mobile", name: "Mobile web (390×844)", w: 390.0, h: 844.0, frame: "phone" },
    Device { id: "window", name: "Desktop app window (1024×700)", w: 1024.0, h: 700.0, frame: "window" },
    Device { id: "watch", name: "Watch (198×242)", w: 198.0, h: 242.0, frame: "watch" },
    Device { id: "watch_round", name: "Round watch (240×240)", w: 240.0, h: 240.0, frame: "watch_round" },
    Device { id: "tv", name: "TV (1920×1080)", w: 1920.0, h: 1080.0, frame: "tv" },
    Device { id: "car", name: "Car display (1280×480)", w: 1280.0, h: 480.0, frame: "car" },
    Device { id: "spatial", name: "Spatial / VR window (1280×720)", w: 1280.0, h: 720.0, frame: "spatial" },
    Device { id: "none", name: "Free canvas (1280×900)", w: 1280.0, h: 900.0, frame: "none" },
];

pub fn device(id: &str) -> Option<&'static Device> {
    DEVICES.iter().find(|d| d.id == id)
}

pub static ICONS: &[&str] = &[
    "home", "search", "user", "users", "settings", "heart", "star", "bell", "menu", "close", "plus", "minus", "check",
    "chevron_left", "chevron_right", "chevron_down", "chevron_up", "arrow_left", "arrow_right", "arrow_up", "arrow_down",
    "cart", "mail", "camera", "share", "trash", "edit", "lock", "calendar", "chat", "play", "pause", "image", "pin",
    "phone", "download", "upload", "filter", "more", "more_v", "info", "warning", "globe", "bookmark", "clock", "wifi",
    "battery", "mic", "grid", "list", "sun", "moon", "cloud", "folder", "file", "link", "eye", "refresh", "send",
    "sparkles", "logout", "help", "bolt", "music", "video", "tag", "gift", "card", "thumbs_up",
];

pub fn is_icon(s: &str) -> bool {
    ICONS.contains(&s)
}
