# wifishare for Android

Send files between your Android phone and the computers running [wifishare](../../wifishare)
(Mac and Ubuntu), over your Wi-Fi.

- **Share menu:** Share → *wifishare: send to computer* → pick a computer. The files land in its
  Downloads folder.
- **Browse the computer:** the folders it shares (chosen in its Files tab). Select files or
  folders and tap *Save to phone* to copy them into Download. In a folder that allows it,
  *Send files here* copies files from the phone into it.
- **Be browsed:** the computer sees all of this phone's storage as "Phone" (in its Files tab) and
  can copy from it, and to it unless *Let computers add files* is off. Nothing is ever
  overwritten or deleted.
- **One computer at a time:** tap the name at the top to switch, or to pair another one.
- **Stays linked** in the background (with a notification), so the computer can browse the
  phone and copies finish while the app is closed. Settings can turn that off.

Copies survive Wi-Fi drops: they pause and carry on from the bytes already copied.

## Set up

1. On the computer, run wifishare and start sharing (either mode). Phones connect on port 24901;
   on Ubuntu with a firewall, `sudo ufw allow 24901`.
2. Install the app (`app-release.apk`, see below). Open it, tap *Pair a computer*, pick the
   computer (or type its address) and check that both screens show the same 6-digit code.
   Confirm on the phone and press *Allow* on the computer.
3. In the app's Settings, tap *Allow access to all files*. Without it the computer cannot browse
   the phone, and copies are saved in the app's own folder instead of Download.

## Build

Needs JDK 17, the Android SDK (platform 35) with the NDK, Rust with the Android targets, and
cargo-ndk:

```sh
rustup target add aarch64-linux-android x86_64-linux-android
cargo install cargo-ndk
export ANDROID_HOME=~/Library/Android/sdk     # wherever your SDK is
./gradlew :app:assembleRelease                # app/build/outputs/apk/release/app-release.apk
./gradlew :app:installDebug                   # or straight onto a connected phone
```

Gradle builds the Rust engine (`rust/`, a JNI wrapper around `wifishare::mobile`) with cargo-ndk
for arm64 phones and x86_64 emulators before it packages the app. Release builds are signed
with your local debug key, so later updates must be built on the same machine.

## How it works

The app is a thin Kotlin/Compose layer over the same Rust networking code the computers run:
Noise-encrypted links, pinned keys from pairing, LAN discovery, and the shared-folder protocol.
The phone never listens; it dials the computer's phone port for the control link, for its own
requests, and when the computer asks to browse it (a dial-back over the control link). Files
shared from other apps are first copied into the app's cache, because the engine sends paths and
a shared item may stop being readable once the other app moves on.

## Limits

- Files the computer copies onto the phone show up in file managers at once, but the gallery
  may only pick them up after its next media scan. Copies you save from the computer are
  scanned straight away.
- No clipboard or notification sync; files only.
- Tested on the Android 15 emulator against stand-ins for the computer side, not yet on a real
  phone.
