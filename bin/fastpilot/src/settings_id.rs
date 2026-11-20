//! `fastpilot config list` for people, in Indonesian (F9). The `config` MCP tool keeps the English
//! of `fastpilot_core::config::describe` (messages for the AI stay English).

use fastpilot_core::config::{self, Rule, Source, SETTINGS};

/// What each setting does, in the words of the Batasan page.
pub fn help(key: &str) -> Option<&'static str> {
    Some(match key {
        "allowed_domains" => "sesi web hanya membuka halaman dari host ini (`*.contoh.com` untuk subdomain); kosong = semua",
        "confirm_actions" => "tap pada elemen bernama begini butuh `confirm: true`; belum diisi: hapus, delete, remove (bawaan); `default` = delete, pay, buy, transfer, …; [] = tanpa konfirmasi",
        "vars" => "Secret: variabel env yang boleh dibaca flow sebagai ${NAMA}; nilainya tampil sebagai ***",
        "state_expire_days" => "login tersimpan ditolak setelah sekian hari (bawaan 30)",
        "block_actions" => "tap pada elemen yang teksnya berisi kata ini selalu ditolak, juga dengan `confirm: true`; belum diisi: daftar bawaan (bayar, pay, beli, buy, checkout, purchase, transfer, buat pesanan, place order, hapus akun, delete account; per kata utuh), juga `default` di daftar; [] = tidak ada",
        "allow_app_install" => "langkah installApp / uninstallApp boleh jalan (bawaan: boleh)",
        "allow_app_reset" => "clearState / clearKeychain boleh menghapus data app (bawaan: boleh)",
        "allow_physical_devices" => "sesi boleh memakai HP fisik; false = hanya emulator dan simulator (bawaan: boleh)",
        "allow_device_settings" => "langkah android/ios boleh mengubah setting HP: setNetwork, setLocation, setPermissions, setLocale (bawaan: boleh)",
        "allow_eval_script" => "langkah evalScript boleh menjalankan JavaScript di halaman; belum diisi: mati selama block_actions aktif (script bisa menekan tombol yang diblokir), boleh kalau daftar dikosongkan; `true` = selalu boleh (hanya orang)",
        "allow_takeover" => "user boleh Ambil alih sesi di `fastpilot ui`; matikan untuk agent yang punya shell (bawaan: boleh)",
        "upload_dirs" => "uploadFile hanya mengambil file di folder ini; kosong = mana saja",
        "max_steps" => "`fastpilot run`: panggilan tool AI per test (bawaan 30)",
        "test_timeout" => "`fastpilot run`: menit untuk satu test sebelum dihentikan dan gagal (bawaan: tanpa batas)",
        "headful" => "tampilkan jendela browser (bawaan false: tanpa jendela, tidak mengganggu)",
        "model" => "`fastpilot run`: satu model Claude untuk semua test",
        "model_replay" => "`fastpilot run`: model yang mengulang path memory (bawaan claude-haiku-5-5)",
        "model_explore" => "`fastpilot run`: model yang menjelajah (bawaan claude-opus-5-5)",
        "flow_timeout" => "detik untuk satu run_flow (bawaan 300)",
        "action_timeout" => "detik untuk satu aksi (tap, ketik, pilih, scroll) (bawaan 5); mengetik di HP mendapat tiga kali lipat untuk keyboard",
        "retry_budget" => "`fastpilot run`: jumlah ulang test gagal per suite (bawaan 3)",
        "breaker" => "`fastpilot run`: kegagalan infra berturut-turut sebelum sisanya dilewati (bawaan 3)",
        "android_serial" => "serial device atau emulator Android bawaan",
        "ios_device" => "simulator yang dinyalakan kalau belum ada yang jalan (bawaan iPhone 16)",
        "ios_udid" => "UDID simulator atau device iOS bawaan",
        "memory" => "path memory: rw (bawaan), readonly, off",
        "daemon_idle" => "daemon CLI shell berhenti setelah sekian detik menganggur (bawaan 1800)",
        "chrome" => "file Chrome/Chromium",
        "claude" => "`fastpilot run`: CLI claude yang dipakai",
        "provider" => "`fastpilot run`: siapa yang menjalankan test: claude (Claude Code, bawaan), atau API OpenAI-compatible: openai, openrouter, groq, ollama, custom (key: `fastpilot config set-key <provider>`)",
        "provider_base_url" => "base URL API OpenAI-compatible (bawaan per provider; wajib untuk custom), mis. https://api.openai.com/v1",
        "provider_model" => "id model di provider OpenAI-compatible (wajib kalau provider bukan claude)",
        "dir" => "tempat run, memory, state, dan baseline disimpan (bawaan ./.fastpilot)",
        "ios_runner" => "folder build runner iOS (bawaan ios-runner/build)",
        "adb_server" => "adb server untuk device Android di mesin lain, host:port (mis. 127.0.0.1:15037 lewat tunnel SSH)",
        "chrome_endpoint" => "Chrome jarak jauh untuk sesi web: http://host:port atau ws://... (lewat tunnel SSH; jangan buka port-nya)",
        _ => return None,
    })
}

/// Every setting with its value now, where it comes from, and who may change it.
pub fn describe() -> String {
    let mut lines = vec![format!("pengaturan ({}; variabel env menang)", config::path().display())];
    for s in SETTINGS {
        let shown = |v: &str| if v.is_empty() { "[]".to_owned() } else { v.to_owned() };
        let value = match config::effective(s.key) {
            Source::Env(v) => format!("{} (env FASTPILOT_{})", shown(&v), s.key.to_uppercase()),
            Source::File(v) => format!("{} (file)", shown(&v)),
            Source::Default => match s.key {
                "block_actions" => format!("{} (default (bawaan))", fastpilot_core::policy::DEFAULT_BLOCKED.join(", ")),
                "confirm_actions" => format!("{} (default (bawaan))", fastpilot_core::policy::DEFAULT_CONFIRM.join(", ")),
                "allow_eval_script" => "mati selama block_actions aktif, selain itu boleh (default (bawaan))".to_owned(),
                _ => "— (bawaan)".to_owned(),
            },
        };
        let who = match s.rule {
            Rule::Free => "",
            Rule::TightenOnly => " [AI: hanya memperketat]",
            Rule::HumanOnly => " [hanya orang]",
        };
        lines.push(format!("{:<18} {value}{who}\n{:<18} {}", s.key, "", help(s.key).unwrap_or(s.help)));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    /// F9: every setting has its Indonesian line, so `config list` has no English for people.
    #[test]
    fn every_setting_has_indonesian_help() {
        for s in fastpilot_core::config::SETTINGS {
            assert!(super::help(s.key).is_some(), "no Indonesian help for {}", s.key);
        }
    }
}
