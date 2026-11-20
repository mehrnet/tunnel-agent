use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    process::Command,
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};

const SSH_KEY_PREFIX: &[u8] = b"\0\0\0\x0bssh-ed25519\0\0\0\x20";
const HPN_PRIVSEP_DIR: &str = "/var/empty";
const HPN_PRIVSEP_USER: &str = "hpnsshd";

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TransportKeys {
    pub(crate) hpnssh: Option<String>,
    pub(crate) rathole: Option<String>,
}

#[derive(Deserialize, Serialize)]
pub(crate) struct NoiseIdentity {
    pub(crate) private: String,
    pub(crate) public: String,
}

pub(crate) fn valid_ssh_key(key: &str) -> bool {
    key.len() == 68
        && STANDARD.decode(key).is_ok_and(|raw| {
            raw.starts_with(SSH_KEY_PREFIX) && raw.len() == SSH_KEY_PREFIX.len() + 32
        })
}

pub(crate) fn valid_noise_key(key: &str) -> bool {
    key.len() == 44
        && STANDARD
            .decode(key)
            .is_ok_and(|raw| raw.len() == 32 && raw.iter().any(|byte| *byte != 0))
}

pub(crate) fn hpn_path(state: &Path) -> PathBuf {
    state.join("transport-identities").join("hpnssh_ed25519")
}

fn noise_path(state: &Path) -> PathBuf {
    state.join("transport-identities").join("rathole.json")
}

pub(crate) fn ssh_public_key(state: &Path) -> Result<String, String> {
    let raw = fs::read_to_string(hpn_path(state).with_extension("pub"))
        .map_err(|error| format!("could not read local HPNSSH public key: {error}"))?;
    let mut fields = raw.split_whitespace();
    let algorithm = fields.next();
    fields
        .next()
        .filter(|key| algorithm == Some("ssh-ed25519") && valid_ssh_key(key))
        .map(str::to_owned)
        .ok_or_else(|| "invalid local HPNSSH public key".to_owned())
}

pub(crate) fn read_noise_identity(state: &Path) -> Result<NoiseIdentity, String> {
    let raw = fs::read(noise_path(state))
        .map_err(|error| format!("could not read local Rathole key: {error}"))?;
    let identity: NoiseIdentity = serde_json::from_slice(&raw)
        .map_err(|_| "invalid local Rathole identity file".to_owned())?;
    if !valid_noise_key(&identity.private) || !valid_noise_key(&identity.public) {
        return Err("invalid local Rathole identity keys".to_owned());
    }
    Ok(identity)
}

pub(crate) fn provision(bundle: &Path, state: &Path) -> TransportKeys {
    let dir = state.join("transport-identities");
    if let Err(error) = fs::create_dir_all(&dir)
        .and_then(|()| fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)))
    {
        eprintln!("transport identity directory unavailable: {error}");
        return TransportKeys::default();
    }
    let hpnssh = report(provision_ssh(bundle, state));
    let rathole = report(provision_noise(bundle, state));
    TransportKeys { hpnssh, rathole }
}

fn report(result: Result<String, String>) -> Option<String> {
    result
        .map_err(|error| eprintln!("transport identity unavailable: {error}"))
        .ok()
}

fn provision_ssh(bundle: &Path, state: &Path) -> Result<String, String> {
    ensure_hpn_baseline()?;
    let path = hpn_path(state);
    if !path.is_file() {
        let output = Command::new(bundle.join("hpnssh/bin/hpnssh-keygen"))
            .args(["-q", "-t", "ed25519", "-N", "", "-f"])
            .arg(&path)
            .output()
            .map_err(|error| format!("could not start HPNSSH keygen: {error}"))?;
        if !output.status.success() {
            return Err("HPNSSH keygen failed".to_owned());
        }
    }
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("could not secure local HPNSSH identity: {error}"))?;
    ssh_public_key(state)
}

fn ensure_hpn_baseline() -> Result<(), String> {
    fs::create_dir_all(HPN_PRIVSEP_DIR)
        .map_err(|error| format!("HPNSSH privilege separation directory unavailable: {error}"))?;
    let dir = fs::symlink_metadata(HPN_PRIVSEP_DIR).map_err(|error| {
        format!("cannot inspect HPNSSH privilege separation directory: {error}")
    })?;
    if !dir.is_dir() || dir.uid() != 0 || dir.mode() & 0o022 != 0 {
        return Err("HPNSSH privilege separation directory is not protected".to_owned());
    }
    if account_uid()?.is_none() {
        create_hpn_account()?;
    }
    if !account_uid()?.is_some_and(|uid| uid != 0) {
        return Err("HPNSSH requires an unprivileged hpnsshd system account".to_owned());
    }
    Ok(())
}

fn account_uid() -> Result<Option<libc::uid_t>, String> {
    let mut entry = std::mem::MaybeUninit::<libc::passwd>::uninit();
    let mut buffer = [0u8; 16_384];
    let mut found = std::ptr::null_mut();
    let status = unsafe {
        libc::getpwnam_r(
            c"hpnsshd".as_ptr(),
            entry.as_mut_ptr(),
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut found,
        )
    };
    if status != 0 {
        return Err(format!(
            "could not query HPNSSH system account: {}",
            std::io::Error::from_raw_os_error(status)
        ));
    }
    Ok((!found.is_null()).then(|| unsafe { entry.assume_init().pw_uid }))
}

fn create_hpn_account() -> Result<(), String> {
    if unsafe { libc::geteuid() } != 0 {
        return Err("root is needed to initialize the HPNSSH system account".to_owned());
    }
    let status = Command::new("useradd")
        .args([
            "--system",
            "--no-create-home",
            "--home-dir",
            HPN_PRIVSEP_DIR,
            "--shell",
            "/bin/false",
            HPN_PRIVSEP_USER,
        ])
        .status();
    let status = match status {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Command::new("adduser")
            .args([
                "-S",
                "-D",
                "-H",
                "-h",
                HPN_PRIVSEP_DIR,
                "-s",
                "/bin/false",
                HPN_PRIVSEP_USER,
            ])
            .status(),
        other => other,
    };
    if status.is_ok_and(|status| status.success()) || account_uid()?.is_some() {
        Ok(())
    } else {
        Err(
            "could not create HPNSSH privilege separation account with useradd or adduser"
                .to_owned(),
        )
    }
}

fn provision_noise(bundle: &Path, state: &Path) -> Result<String, String> {
    let path = noise_path(state);
    if !path.is_file() {
        let output = Command::new(bundle.join("bin/rathole"))
            .arg("--genkey")
            .output()
            .map_err(|error| format!("could not start Rathole keygen: {error}"))?;
        if !output.status.success() {
            return Err("Rathole keygen failed".to_owned());
        }
        let stdout = std::str::from_utf8(&output.stdout)
            .map_err(|_| "invalid Rathole keygen response".to_owned())?;
        let mut keys = stdout.lines().filter(|line| valid_noise_key(line));
        let identity = match (keys.next(), keys.next(), keys.next()) {
            (Some(private), Some(public), None) => NoiseIdentity {
                private: private.to_owned(),
                public: public.to_owned(),
            },
            _ => return Err("Rathole keygen did not return a key pair".to_owned()),
        };
        let json = serde_json::to_string(&identity)
            .map_err(|_| "could not encode Rathole identity".to_owned())?;
        crate::manifest::write_config_atomic(&path, &json)?;
    }
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("could not secure local Rathole identity: {error}"))?;
    read_noise_identity(state).map(|identity| identity.public)
}
