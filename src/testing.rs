use crate::config::Arch;
use crate::config::Config;
use crate::config::Cpu;
use crate::config::Firmware;
use crate::config::Ids;
use crate::config::Secret;
use crate::config::Source;
use crate::config::Unlock;
use crate::config::User;
use crate::layout::Crypt;
use crate::layout::Layout;
use crate::network::Backend;
use crate::network::Network;
use uuid::Uuid;

pub fn config() -> Config {
    Config {
        layout: Layout::simple("/dev/sda", "/dev/sda", Firmware::Uefi).with_root_crypt(Some(
            Crypt {
                unlock: Unlock::Typed,
                detached: false,
            },
        )),
        firmware: Firmware::Uefi,
        source: Source {
            repos: vec!["evillin".into()],
            ..Source::default()
        },
        arch: Arch::X86_64,
        cpu: Cpu::Other,
        hostname: "moon".into(),
        users: vec![User {
            name: "alice".into(),
            password: Secret::new("pw"),
            hashed: false,
        }],
        passphrase: Some(Secret::new("phrase")),
        keys: Vec::new(),
        uboot: None,
        timezone: "Europe/Moscow".into(),
        initrd_network: Network::None,
        system_network: Network::None,
        backend: Backend::Networkd,
        ssh_keys: Vec::new(),
        kernel_args: Vec::new(),
        services: Vec::new(),
    }
}

pub fn ids() -> Ids {
    ids_for(&config().layout)
}

pub fn ids_for(layout: &Layout) -> Ids {
    let mut counter = 0;
    let mut ids = Ids::generate(layout, || {
        counter += 1;
        Uuid::from_u128(counter)
    });

    ids.fat = 0xABCD_1234;
    ids
}

pub fn app_on_disk_page(draft: crate::draft::Draft) -> crate::app::App {
    let mut app = crate::app::App::new(draft);

    app.page = crate::draft::Page::ALL
        .iter()
        .position(|page| *page == crate::draft::Page::Disk)
        .unwrap();
    app
}
