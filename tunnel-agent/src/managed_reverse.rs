use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use serde::Deserialize;

use crate::{
    manifest::{
        GeneratedFile, Listener, ListenerClaim, ProcessSpec, RestartPolicy, TunnelListenerScheme,
    },
    transport_identity,
};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ManagedTransport {
    Hpnssh,
    Rathole,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ManagedRole {
    Server,
    Client,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ManagedReverse {
    pub(crate) id: String,
    pub(crate) enabled: bool,
    pub(crate) transport: ManagedTransport,
    pub(crate) role: ManagedRole,
    pub(crate) control_port: u16,
    pub(crate) listener: Listener,
    pub(crate) scheme: TunnelListenerScheme,
    pub(crate) peer_host: String,
    pub(crate) peer_key: String,
    pub(crate) local_key: String,
    generation: String,
}

impl ManagedReverse {
    pub(crate) fn artifact(&self) -> &'static str {
        match (self.transport, self.role) {
            (ManagedTransport::Rathole, _) => "rathole",
            (ManagedTransport::Hpnssh, ManagedRole::Server) => "hpnsshd",
            (ManagedTransport::Hpnssh, ManagedRole::Client) => "hpnssh",
        }
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        let valid_key = match self.transport {
            ManagedTransport::Hpnssh => transport_identity::valid_ssh_key,
            ManagedTransport::Rathole => transport_identity::valid_noise_key,
        };
        if !valid_key(&self.peer_key)
            || !valid_key(&self.local_key)
            || self.peer_key == self.local_key
        {
            return Err(
                "managed reverse tunnel requires distinct pinned transport identities".to_owned(),
            );
        }
        if self.id.len() > 80
            || !valid_host(&self.peer_host)
            || self.generation.len() != 64
            || !self.generation.bytes().all(|byte| byte.is_ascii_hexdigit())
            || self.listener.bind != "127.0.0.1"
            || self.control_port < 1024
            || self.listener.port < 1024
            || self.listener.port == self.control_port
        {
            return Err("invalid managed reverse tunnel listener or peer settings".to_owned());
        }
        if self.transport == ManagedTransport::Rathole && self.scheme != TunnelListenerScheme::Tcp {
            return Err("managed Rathole supports fixed TCP listeners only".to_owned());
        }
        match (&self.listener.target, self.role, self.scheme) {
            (Some(target), ManagedRole::Client, TunnelListenerScheme::Tcp)
                if valid_host(&target.host) && target.port != 0 =>
            {
                Ok(())
            }
            (None, ManagedRole::Server, _)
            | (None, ManagedRole::Client, TunnelListenerScheme::Socks5) => Ok(()),
            _ => Err("invalid managed reverse tunnel target".to_owned()),
        }
    }

    pub(crate) fn compile(
        &self,
        binary: &Path,
        bundle: &Path,
        state: &Path,
        check_only: bool,
    ) -> Result<ManagedPlan, String> {
        let files = state.join("generated");
        let (mut args, generated) = match self.transport {
            ManagedTransport::Hpnssh => self.hpn_files(state, &files, check_only)?,
            ManagedTransport::Rathole => self.rathole_files(state, &files, check_only)?,
        };
        let program =
            if self.transport == ManagedTransport::Hpnssh && self.role == ManagedRole::Server {
                args.extend([
                    "hpn-server-worker".to_owned(),
                    "--binary".to_owned(),
                    binary.display().to_string(),
                    "--config".to_owned(),
                    generated[0].path.display().to_string(),
                ]);
                bundle.join("bin/tunnel-agent").display().to_string()
            } else {
                binary.display().to_string()
            };
        let process = ProcessSpec {
            id: self.id.clone(),
            argv: std::iter::once(program).chain(args).collect(),
            env: BTreeMap::new(),
            workdir: None,
            restart: RestartPolicy::default(),
        };
        Ok(ManagedPlan {
            process,
            generated,
            listeners: self.claims(),
        })
    }

    pub(crate) fn matches_identity(&self, keys: &transport_identity::TransportKeys) -> bool {
        (match self.transport {
            ManagedTransport::Hpnssh => keys.hpnssh.as_deref(),
            ManagedTransport::Rathole => keys.rathole.as_deref(),
        }) == Some(self.local_key.as_str())
    }

    fn claims(&self) -> Vec<ListenerClaim> {
        if self.role == ManagedRole::Client {
            return Vec::new();
        }
        [
            ("0.0.0.0", self.control_port),
            ("127.0.0.1", self.listener.port),
        ]
        .map(|(bind, port)| ListenerClaim {
            owner: self.id.clone(),
            bind: bind.to_owned(),
            port,
        })
        .into()
    }

    fn hpn_files(
        &self,
        state: &Path,
        dir: &Path,
        check_only: bool,
    ) -> Result<(Vec<String>, Vec<GeneratedFile>), String> {
        let identity = transport_identity::hpn_path(state);
        if !check_only
            && (!identity.is_file() || transport_identity::ssh_public_key(state)? != self.local_key)
        {
            return Err(
                "managed HPNSSH lane local identity does not match its registered key".to_owned(),
            );
        }
        Ok(match self.role {
            ManagedRole::Server => (Vec::new(), self.hpn_server(&identity, dir)),
            ManagedRole::Client => self.hpn_client(&identity, dir),
        })
    }

    fn hpn_server(&self, identity: &Path, dir: &Path) -> Vec<GeneratedFile> {
        let config = dir.join(format!("{}.sshd_config", self.id));
        let auth = dir.join(format!("{}.authorized_keys", self.id));
        let contents = format!(
            concat!(
                "# Generation {}\nPort {}\nAddressFamily inet\nListenAddress 0.0.0.0\n",
                "HostKey \"{}\"\nAuthorizedKeysFile \"{}\"\nPidFile none\nAllowUsers root\n",
                "PermitRootLogin prohibit-password\nAuthenticationMethods publickey\n",
                "PasswordAuthentication no\nKbdInteractiveAuthentication no\n",
                "AllowTcpForwarding remote\nPermitListen 127.0.0.1:{}\nPermitOpen none\n",
                "AllowStreamLocalForwarding no\nGatewayPorts no\nPermitTunnel no\n",
                "PermitTTY no\nX11Forwarding no\nAllowAgentForwarding no\n",
                "MaxSessions 0\nUseDNS no\nClientAliveInterval 20\nClientAliveCountMax 3\n"
            ),
            self.generation,
            self.control_port,
            quoted_path(identity),
            quoted_path(&auth),
            self.listener.port
        );
        let key = format!(
            "restrict,port-forwarding,permitlisten=\"127.0.0.1:{}\" ssh-ed25519 {}\n",
            self.listener.port, self.peer_key
        );
        vec![file(config, contents), file(auth, key)]
    }

    fn hpn_client(&self, identity: &Path, dir: &Path) -> (Vec<String>, Vec<GeneratedFile>) {
        let hosts = dir.join(format!("{}.known_hosts", self.id));
        let mut forward = format!("127.0.0.1:{}", self.listener.port);
        if let Some(target) = &self.listener.target {
            forward.push_str(&format!(":{}:{}", target.host, target.port));
        }
        let contents = format!(
            "[{}]:{} ssh-ed25519 {} {}\n",
            self.peer_host, self.control_port, self.peer_key, self.generation
        );
        let mut args = [
            "-N",
            "-F",
            "/dev/null",
            "-o",
            "BatchMode=yes",
            "-o",
            "IdentitiesOnly=yes",
            "-o",
            "ExitOnForwardFailure=yes",
            "-o",
            "ConnectTimeout=10",
            "-o",
            "ServerAliveInterval=20",
            "-o",
            "ServerAliveCountMax=3",
            "-o",
            "StrictHostKeyChecking=yes",
            "-o",
            "GlobalKnownHostsFile=/dev/null",
            "-o",
        ]
        .map(str::to_owned)
        .to_vec();
        args.extend([
            format!("UserKnownHostsFile={}", hosts.display()),
            "-i".to_owned(),
            identity.display().to_string(),
            "-p".to_owned(),
            self.control_port.to_string(),
            "-R".to_owned(),
            forward,
            format!("root@{}", self.peer_host),
        ]);
        (args, vec![file(hosts, contents)])
    }

    fn rathole_files(
        &self,
        state: &Path,
        dir: &Path,
        check_only: bool,
    ) -> Result<(Vec<String>, Vec<GeneratedFile>), String> {
        let private_key = if check_only {
            String::new()
        } else {
            let identity = transport_identity::read_noise_identity(state)?;
            if identity.public != self.local_key {
                return Err(
                    "managed Rathole lane local identity does not match its registered key"
                        .to_owned(),
                );
            }
            identity.private
        };
        let (role, endpoint, service) = match self.role {
            ManagedRole::Server => (
                "server",
                format!("bind_addr = \"0.0.0.0:{}\"", self.control_port),
                format!("bind_addr = \"127.0.0.1:{}\"", self.listener.port),
            ),
            ManagedRole::Client => {
                let target = self
                    .listener
                    .target
                    .as_ref()
                    .expect("validated managed Rathole target");
                (
                    "client",
                    format!("remote_addr = \"{}:{}\"", self.peer_host, self.control_port),
                    format!("local_addr = \"{}:{}\"", target.host, target.port),
                )
            }
        };
        let contents = format!(
            "[{role}]\n{endpoint}\n\n[{role}.transport]\ntype = \"noise\"\n\n[{role}.transport.noise]\npattern = \"Noise_KK_25519_ChaChaPoly_BLAKE2s\"\nlocal_private_key = \"{}\"\nremote_public_key = \"{}\"\n\n[{role}.services.lane]\ntoken = \"{}\"\n{service}\n",
            private_key, self.peer_key, self.generation,
        );
        let config = dir.join(format!("{}.rathole.toml", self.id));
        Ok((
            vec![format!("--{role}"), config.display().to_string()],
            vec![file(config, contents)],
        ))
    }
}

pub(crate) struct ManagedPlan {
    pub(crate) process: ProcessSpec,
    pub(crate) generated: Vec<GeneratedFile>,
    pub(crate) listeners: Vec<ListenerClaim>,
}

fn file(path: PathBuf, content: String) -> GeneratedFile {
    GeneratedFile { path, content }
}

fn quoted_path(path: &Path) -> String {
    path.display()
        .to_string()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

pub(crate) fn valid_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label.as_bytes()[0].is_ascii_alphanumeric()
                && label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}

#[cfg(test)]
#[path = "managed_reverse_tests.rs"]
mod tests;
