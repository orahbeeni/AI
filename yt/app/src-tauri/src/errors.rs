//! Translate yt-dlp failures into plain language with a suggested fix.

#[derive(Debug, Clone, PartialEq)]
pub struct Explained {
    pub message: String,
    pub hint: String,
}

pub fn explain(raw: &str) -> Option<Explained> {
    let l = raw.to_lowercase();
    let e = |m: &str, h: &str| Some(Explained { message: m.into(), hint: h.into() });

    if l.contains("not a bot") || l.contains("sign in to confirm you") && !l.contains("age") {
        return e(
            "YouTube wants proof you are not a bot.",
            "Open Options → Network & sign-in and choose your browser under “Use cookies from”, then retry.",
        );
    }
    if l.contains("confirm your age") || l.contains("age-restricted") || l.contains("age restricted") {
        return e(
            "This video is age-restricted.",
            "Sign in to YouTube in your browser, then choose that browser under “Use cookies from” in Options → Network & sign-in.",
        );
    }
    if l.contains("members-only") || l.contains("join this channel") || l.contains("members only") {
        return e(
            "This video is for channel members only.",
            "Use cookies from a browser that is signed in to an account with membership.",
        );
    }
    if l.contains("private video") {
        return e("This video is private.", "Only the owner can share it. If it was shared with you, use cookies from your signed-in browser.");
    }
    if l.contains("video unavailable") || l.contains("has been removed") || l.contains("no longer available") {
        return e("This video is unavailable.", "It may have been removed or made private. Check the link in your browser.");
    }
    if l.contains("not available in your country") || l.contains("geo") && l.contains("restrict") {
        return e("This video is blocked in your region.", "Try a VPN or set a proxy in Options → Network & sign-in.");
    }
    if l.contains("javascript runtime") || l.contains("js runtime") {
        return e("A JavaScript runtime is needed for YouTube.", "Open Settings → Tools and install “deno”.");
    }
    if l.contains("http error 403") {
        return e(
            "The site refused the download (403).",
            "Update yt-dlp in Settings → Tools and make sure deno is installed. If it persists, try cookies from your browser.",
        );
    }
    if l.contains("http error 429") || l.contains("too many requests") {
        return e("Too many requests — the site is rate-limiting you.", "Wait a while, or set a download speed limit in Options → Network & sign-in.");
    }
    if l.contains("ffmpeg") && (l.contains("not found") || l.contains("not installed") || l.contains("could not find")) {
        return e("ffmpeg is needed for this option.", "Open Settings → Tools and install ffmpeg.");
    }
    if l.contains("requested format is not available") {
        return e(
            "That quality/format isn’t available for this video.",
            "Choose a different preset, or pick a format yourself from the Formats table.",
        );
    }
    if l.contains("unsupported url") {
        return e("This link isn’t supported.", "Check that you copied the full video, playlist or channel link.");
    }
    if l.contains("no space left") || l.contains("not enough disk space") {
        return e("The disk is full.", "Free some space or choose a different download folder.");
    }
    if l.contains("permission denied") {
        return e("The app can’t write to that folder.", "Choose a download folder you have permission to write to.");
    }
    if l.contains("name resolution") || l.contains("unable to download webpage") || l.contains("network is unreachable") || l.contains("timed out") {
        return e("Couldn’t reach the site.", "Check your internet connection and try again.");
    }
    if l.contains("this live event will begin") || l.contains("premieres in") {
        return e("This video hasn’t started yet.", "Turn on “Wait for it to start” to have the app keep checking.");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bot_check_suggests_cookies() {
        let x = explain("ERROR: [youtube] abc: Sign in to confirm you’re not a bot.").unwrap();
        assert!(x.hint.contains("cookies"));
    }

    #[test]
    fn age_gate_is_not_mistaken_for_bot_check() {
        let x = explain("Sign in to confirm your age. This video may be inappropriate").unwrap();
        assert!(x.message.contains("age"));
    }

    #[test]
    fn unknown_errors_return_none() {
        assert!(explain("something entirely unexpected").is_none());
    }

    #[test]
    fn upcoming_live_event() {
        assert!(explain("This live event will begin in 2 hours").unwrap().hint.contains("Wait"));
    }
}
