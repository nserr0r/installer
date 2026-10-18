use std::net::Ipv4Addr;

use crate::error::Error;
use crate::i18n::tr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manual {
    pub address: Ipv4Addr,
    pub prefix: u8,
    pub gateway: Ipv4Addr,
    pub dns: Vec<Ipv4Addr>,
    pub interface: String,
}

impl Manual {
    pub fn netmask(&self) -> Ipv4Addr {
        Ipv4Addr::from(
            u32::MAX
                .checked_shl(32 - u32::from(self.prefix))
                .unwrap_or(0),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Network {
    None,
    Dhcp { interface: Option<String> },
    Manual(Manual),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Networkd,
    NetworkManager,
}

pub struct Setup {
    pub path: &'static str,
    pub contents: String,
    pub mode: u32,
    pub services: Vec<&'static str>,
    pub masked: Vec<&'static str>,
}

impl Network {
    pub fn validate(&self) -> Result<(), Error> {
        if let Some(interface) = self.interface()
            && (interface.is_empty()
                || interface.len() > 15
                || !interface
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)))
        {
            return Err(Error::Invalid(tr!(
                "invalid interface name: {interface}",
                "неверное имя интерфейса: {interface}"
            )));
        }

        match self {
            Network::Manual(manual) if manual.prefix > 32 => Err(Error::Invalid(tr!(
                "invalid prefix length: {}",
                "неверная длина префикса: {}",
                manual.prefix
            ))),
            _ => Ok(()),
        }
    }

    /// Аргументы командной строки ядра, поднимающие сеть в initramfs (формат `ip=` из dracut.cmdline).
    pub fn cmdline(&self, hostname: &str) -> Option<String> {
        match self {
            Network::None => None,
            Network::Dhcp { interface: None } => Some("rd.neednet=1 ip=dhcp".into()),
            Network::Dhcp {
                interface: Some(interface),
            } => Some(format!("rd.neednet=1 ip={interface}:dhcp")),
            Network::Manual(manual) => {
                let mut text = format!(
                    "rd.neednet=1 ip={}::{}:{}:{hostname}:{}:none",
                    manual.address,
                    manual.gateway,
                    manual.netmask(),
                    manual.interface
                );

                for server in &manual.dns {
                    text.push_str(&format!(" nameserver={server}"));
                }

                Some(text)
            }
        }
    }

    /// Файл настройки сети установленной системы и службы, которые надо включить.
    pub fn setup(&self, backend: Backend) -> Option<Setup> {
        match (self, backend) {
            (Network::None, _) => None,
            (network, Backend::Networkd) => Some(network.networkd()),
            (network, Backend::NetworkManager) => Some(network.network_manager()),
        }
    }

    fn interface(&self) -> Option<&str> {
        match self {
            Network::None => None,
            Network::Dhcp { interface } => interface.as_deref(),
            Network::Manual(manual) => Some(&manual.interface),
        }
    }

    fn networkd(&self) -> Setup {
        let name = self.interface().unwrap_or("en* eth*");
        let mut text = format!("[Match]\nName={name}\n\n[Network]\n");
        let mut services = vec!["systemd-networkd"];

        match self {
            Network::Manual(manual) => {
                text.push_str(&format!(
                    "Address={}/{}\nGateway={}\n",
                    manual.address, manual.prefix, manual.gateway
                ));

                for server in &manual.dns {
                    text.push_str(&format!("DNS={server}\n"));
                }

                if !manual.dns.is_empty() {
                    services.push("systemd-resolved");
                }
            }
            _ => text.push_str("DHCP=yes\n"),
        }

        Setup {
            path: "/etc/systemd/network/10-moonshine.network",
            contents: text,
            mode: 0o644,
            services,
            masked: vec!["NetworkManager"],
        }
    }

    fn network_manager(&self) -> Setup {
        let mut text = "[connection]\nid=moonshine\ntype=ethernet\n".to_string();

        if let Some(interface) = self.interface() {
            text.push_str(&format!("interface-name={interface}\n"));
        }

        match self {
            Network::Manual(manual) => {
                text.push_str(&format!(
                    "\n[ipv4]\nmethod=manual\naddress1={}/{},{}\n",
                    manual.address, manual.prefix, manual.gateway
                ));

                if !manual.dns.is_empty() {
                    let servers: String = manual
                        .dns
                        .iter()
                        .map(|server| format!("{server};"))
                        .collect();

                    text.push_str(&format!("dns={servers}\n"));
                }
            }
            _ => text.push_str("\n[ipv4]\nmethod=auto\n"),
        }

        text.push_str("\n[ipv6]\nmethod=ignore\n");

        Setup {
            path: "/etc/NetworkManager/system-connections/moonshine.nmconnection",
            contents: text,
            mode: 0o600,
            services: vec!["NetworkManager"],
            masked: vec!["systemd-networkd"],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manual() -> Manual {
        Manual {
            address: "192.168.1.10".parse().unwrap(),
            prefix: 24,
            gateway: "192.168.1.1".parse().unwrap(),
            dns: vec!["1.1.1.1".parse().unwrap(), "8.8.8.8".parse().unwrap()],
            interface: "eth0".into(),
        }
    }

    #[test]
    fn network_arguments_follow_the_dracut_ip_format() {
        assert_eq!(Network::None.cmdline("moon"), None);
        assert_eq!(
            Network::Dhcp { interface: None }.cmdline("moon").unwrap(),
            "rd.neednet=1 ip=dhcp"
        );
        assert_eq!(
            Network::Dhcp {
                interface: Some("enp3s0".into())
            }
            .cmdline("moon")
            .unwrap(),
            "rd.neednet=1 ip=enp3s0:dhcp"
        );
        assert_eq!(
            Network::Manual(manual()).cmdline("moon").unwrap(),
            "rd.neednet=1 ip=192.168.1.10::192.168.1.1:255.255.255.0:moon:eth0:none nameserver=1.1.1.1 nameserver=8.8.8.8"
        );
    }

    #[test]
    fn the_prefix_becomes_a_netmask() {
        let netmask = |prefix| Manual { prefix, ..manual() }.netmask().to_string();

        assert_eq!(netmask(0), "0.0.0.0");
        assert_eq!(netmask(20), "255.255.240.0");
        assert_eq!(netmask(32), "255.255.255.255");
    }

    #[test]
    fn networkd_gets_a_dhcp_or_static_unit() {
        let dhcp = Network::Dhcp { interface: None }
            .setup(Backend::Networkd)
            .unwrap();

        assert_eq!(
            dhcp.contents,
            "[Match]\nName=en* eth*\n\n[Network]\nDHCP=yes\n"
        );
        assert_eq!(dhcp.services, ["systemd-networkd"]);

        let fixed = Network::Manual(manual()).setup(Backend::Networkd).unwrap();

        assert_eq!(
            fixed.contents,
            "[Match]\nName=eth0\n\n[Network]\nAddress=192.168.1.10/24\nGateway=192.168.1.1\nDNS=1.1.1.1\nDNS=8.8.8.8\n"
        );
        assert_eq!(fixed.services, ["systemd-networkd", "systemd-resolved"]);
    }

    #[test]
    fn network_manager_gets_a_private_keyfile() {
        let fixed = Network::Manual(manual())
            .setup(Backend::NetworkManager)
            .unwrap();

        assert_eq!(fixed.mode, 0o600);
        assert_eq!(fixed.services, ["NetworkManager"]);
        assert!(fixed.contents.contains("interface-name=eth0\n"));
        assert!(
            fixed
                .contents
                .contains("address1=192.168.1.10/24,192.168.1.1\n")
        );
        assert!(fixed.contents.contains("dns=1.1.1.1;8.8.8.8;\n"));

        let dhcp = Network::Dhcp { interface: None }
            .setup(Backend::NetworkManager)
            .unwrap();

        assert!(dhcp.contents.contains("method=auto") && !dhcp.contents.contains("interface-name"));
    }

    #[test]
    fn no_network_means_no_files() {
        assert!(Network::None.setup(Backend::Networkd).is_none());
        assert!(Network::None.setup(Backend::NetworkManager).is_none());
    }

    #[test]
    fn interface_names_and_prefixes_are_checked() {
        assert!(
            Network::Dhcp {
                interface: Some("eth0 ip=x".into())
            }
            .validate()
            .is_err()
        );
        assert!(
            Network::Dhcp {
                interface: Some(String::new())
            }
            .validate()
            .is_err()
        );
        assert!(
            Network::Manual(Manual {
                prefix: 33,
                ..manual()
            })
            .validate()
            .is_err()
        );
        assert!(Network::Manual(manual()).validate().is_ok());
    }
}
