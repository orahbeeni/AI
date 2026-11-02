package com.omar.wifishare

/**
 * The Rust engine (android/wifishare/rust, built on the wifishare crate). Calls that can only
 * fail return null on success and the error otherwise; `shares`, `list` and `send` answer
 * `{"ok": ...}` or `{"error": "..."}`. The calls that talk to the computer block, so they run off
 * the main thread.
 */
object Native {
    init {
        System.loadLibrary("wifishare_android")
    }

    external fun start(dir: String, name: String, storage: String?, allowAdd: Boolean): String?
    external fun state(): String?
    external fun pair(target: String): String?
    external fun answer(id: Long, yes: Boolean): String?
    external fun follow(name: String?): String?
    external fun forget(name: String): String?
    external fun shares(): String
    external fun list(share: String, path: String): String
    external fun mkdir(share: String, path: String): String?
    external fun download(share: String, dir: String, names: String, dest: String): String?
    external fun upload(share: String, dir: String, files: String): String?
    external fun send(files: String): String
    external fun cancel(id: Long): String?
    external fun clearFinished(): String?
}
