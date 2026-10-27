# SmsManager

An Android SMS app built to be the default messaging app, with spam blocking as the main feature.

- Normal SMS: conversation list, threads, new message with contact suggestions, search (with `*` and `?` wildcards), delete, notifications with inline reply and mark-as-read, retry for failed sends, "reply with message" from the call screen.
- **Select and copy part of a message**: touch and hold a word in any message, drag the handles, tap Copy. Tapping a message opens options for the whole message (copy all, delete, block the number, retry a failed send).
- **Blocking** by sender and by message text, in three match modes: wildcard, loose, or regular expression.
- **Make a rule from a message**: tap the small filter icon beside any incoming message, touch and hold a word and drag the handles to select a phrase, and it becomes a rule. A live line says whether it catches the message and how many messages already in your inbox it matches, so a rule that's too broad is obvious before you save it.
- **Allow rules** that beat block rules, and an option to never block your contacts.
- Blocked messages are never written to your inbox. They sit in the **Blocked** tab for 7/30/90 days or forever, where you can restore them ("Not spam" also adds an allow rule for the sender) or delete them.
- **Copy button for one-time codes**: a message with a code gets a heads-up notification with a "Copy 482913" button (and a Copy button under the message in the thread). Switch it off in Settings.
- **Scan old messages** applies your rules to what's already in the inbox, over 1 week, 1 month, 1 year, 5 years, 10 years or all time, with a preview before anything moves.
- **Backup file** for rules and settings, kept up to date automatically, so an uninstall doesn't cost you your filters.

## Build and run

Needs JDK 17 and the Android SDK (platform 35, build-tools 35). Open the folder in Android Studio, or:

```sh
export ANDROID_HOME=~/Library/Android/sdk   # wherever your SDK is
./gradlew :app:testDebugUnitTest            # rule-matching tests
./gradlew :app:installDebug                 # install on a connected phone/emulator
```

On first launch tap **Make default SMS app** and **Grant permissions** on the Messages tab. Blocking only works while the app is the default, because Android delivers incoming SMS to the default app alone, and it is then up to that app to store them.

## Rule syntax

| Pattern | Meaning |
|---|---|
| `*` | any run of characters, including none |
| `?` | exactly one character |
| `\*` `\?` `\\` | the literal character |

Everything is case-insensitive.

- **Sender rules** match the whole address. `+1555*` blocks every number starting `+1555`; `*4567` blocks numbers ending `4567`; `promo*` blocks alphanumeric senders like `PROMO-DEALS`. Spaces, dashes, dots and parentheses are ignored on both sides, so `+1 (555) 123-*` works.
- **Text rules** match anywhere in the message, so `free gift` blocks "Claim your FREE GIFT now" and `click*http` blocks "click here: http://...". You don't need to add `*` at both ends.
- **Loose** mode uses the same `*` and `?`, but `?` means zero or one character, and spaces, punctuation, accents and case are ignored on both sides. `opt?out` matches "Opt-out", "O.P.T out", "OPT OUT" and "optout". Because separators are removed before matching, a loose pattern can also match across word boundaries (`free gift` matches "FREE-GIFT").
- **Regex** mode is full Java regex (same whole-sender / anywhere-in-text behaviour). Here `?` makes the previous character optional, so use `opt[- ]?out` for the same effect as above.
- Order of evaluation: allow rules, then contacts, then block rules. Use an allow rule like `your code is*` (text) to keep one-time codes flowing from an otherwise blocked sender.
- The Blocked tab has the same search (sender, text and the rule that matched).
- The **Test** button on the Rules tab runs a made-up message through your rules.
- The Messages search box takes the same `*` and `?` (anywhere-in-text, so `lun*noon` finds "Lunch at noon"; `\*` and `\?` are literal).

## Scanning old messages

Rules tab, Scan old: pick a range, the app lists how many received messages would have been blocked (grouped by rule), and "Move to Blocked" copies each into the Blocked tab and then deletes it from the inbox. The copy is made first, so a failure can't lose a message. It needs the default-SMS role, since only that app may delete. Moved messages are kept for the retention period counted from the move, not from when they were sent.

## Backup, export and import

Settings, Backup has two buttons and one switch:

- **Export…** saves your rules and settings once to a file you pick (Downloads, Drive, anywhere), with no ongoing link. Use it to move to another phone or share a rule set.
- **Import…** loads such a file: rules you don't already have are added (duplicates skipped) and settings are replaced. If automatic backup is off, it then offers to keep that file up to date.
- **Keep a backup file up to date** is a switch. Turning it on asks for a file once; after that the file is rewritten a few seconds after every change. Turning it off stops and releases the file.

Blocked messages are never included, since they are private and the file may sit in cloud storage.

- Installing a new version over the old one keeps all data. No backup is needed for that.
- Uninstalling wipes app data. After reinstalling, tap **Import…**, pick your backup file, and say yes to keep it updated.
- Android's own cloud backup is switched off for the app so blocked message text never goes to Google's servers.

## Reporting spam

There is deliberately no in-app reporting. In the US/UK/Canada a carrier short code (7726) can be driven by SMS, and an earlier version did that. South Africa has no equivalent that could be confirmed: spam is reported to WASPA through a web form, and only covers WASPA members, so it can't be automated from the phone.


## Limitations

- **No MMS.** Pictures and group messages are not downloaded or sent. The app registers the MMS receiver that default-SMS eligibility demands and shows a notification when one arrives, but cannot show it. If you rely on MMS, keep that in mind before switching.
- No RCS (not available to third-party apps at all).
- One recipient per message.
- The conversation list scans the whole SMS table, which is fine for thousands of messages but will get slow at hundreds of thousands.
- Dual-SIM: replies use the system default SIM.

## Layout

```
app/src/main/java/com/omar/smsmanager/
  spam/   Rule, PatternMatcher (wildcard/loose -> regex), SpamFilter, Retro (scan old messages)
  sms/    SmsReceiver + Inbound (the receive pipeline), SmsSender, Notifier, MMS stub, respond-via-message
  data/   Db (rules + blocked, SQLite), SmsStore (system SMS provider), ConversationCache, Contacts, Settings, Backup
  ui/     Compose screens
```
