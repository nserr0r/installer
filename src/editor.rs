use std::collections::HashSet;

use ratatui::crossterm::event::KeyCode;

use crate::config::Unlock;
use crate::error::Error;
use crate::i18n::tr;
use crate::i18n::tt;
use crate::input::Input;
use crate::layout;
use crate::layout::Crypt;
use crate::layout::Fs;
use crate::layout::Layout;
use crate::layout::Partition;
use crate::layout::Raid;
use crate::layout::Tuning;
use crate::layout::Use;
use crate::layout::Volume;

const HISTORY: usize = 32;
const PAGE: usize = 10;
/// Каталоги, которые чаще всего выносят на отдельную файловую систему.
const COMMON_MOUNTS: [&str; 9] = [
    "/", "/home", "/var", "/var/log", "/var/lib", "/tmp", "/opt", "/srv", "/usr",
];

/// Готовые настройки файловой системы для клавиши `o`.
struct Preset {
    ext4_mkfs: &'static [&'static str],
    options: &'static [&'static str],
}

impl Preset {
    fn tuning(&self, fs: Fs) -> Tuning {
        let owned = |items: &[&str]| items.iter().map(ToString::to_string).collect();

        Tuning {
            mkfs: match fs {
                Fs::Ext4 => owned(self.ext4_mkfs),
                Fs::Xfs | Fs::Btrfs => Vec::new(),
            },
            options: owned(self.options),
        }
    }
}

const PRESETS: [Preset; 2] = [
    // SSD: без обновления времени доступа и с отдачей освобождённых блоков
    Preset {
        ext4_mkfs: &[],
        options: &["noatime", "discard"],
    },
    // большие файлы: ext4 с 1% резерва и типом largefile4
    Preset {
        ext4_mkfs: &["-T", "largefile4", "-m", "1"],
        options: &["noatime"],
    },
];

/// Что создаёт `n` над выбранным устройством.
enum Layer {
    Volume(String),
    Raid(String),
}

/// Узел дерева: диск или группа LVM раскрываются, остальное — листья.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Node {
    Disk(String),
    Partition(usize),
    Raid(usize),
    Group(String),
    Volume(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub node: Node,
    pub depth: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Item {
    Partition,
    Raid,
    Volume,
}

impl Item {
    pub fn title(self) -> &'static str {
        match self {
            Item::Partition => tt!("partition", "раздел"),
            Item::Raid => tt!("RAID array", "массив RAID"),
            Item::Volume => tt!("LVM volume", "том LVM"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Place {
    Insert(usize),
    Replace(usize),
}

/// Строка ввода для добавления или правки одного элемента.
pub struct Form {
    pub item: Item,
    pub input: Input,
    pub error: Option<String>,
    place: Place,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Stay,
    Close,
}

pub struct Editor {
    layout: Layout,
    pub cursor: usize,
    pub form: Option<Form>,
    folded: HashSet<Node>,
    history: Vec<Layout>,
    disks: Vec<String>,
    changed: bool,
}

impl Editor {
    pub fn new(layout: Layout, disks: Vec<String>) -> Self {
        Self {
            layout,
            cursor: 0,
            form: None,
            folded: HashSet::new(),
            history: Vec::new(),
            disks,
            changed: false,
        }
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    pub fn disks(&self) -> &[String] {
        &self.disks
    }

    /// Были ли правки с прошлого вопроса; вызывающий забирает разметку в черновик.
    pub fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }

    fn groups(&self) -> Vec<String> {
        let mut groups = self.layout.groups();

        for volume in &self.layout.volumes {
            if !groups.contains(&volume.group) {
                groups.push(volume.group.clone());
            }
        }

        groups
    }

    pub fn tree(&self) -> Vec<Entry> {
        let mut tree = Vec::new();
        let mut push = |node, depth| tree.push(Entry { node, depth });

        for disk in self.layout.disks() {
            let node = Node::Disk(disk.clone());
            let open = !self.folded.contains(&node);

            push(node, 0);

            if open {
                for (index, part) in self.layout.partitions.iter().enumerate() {
                    if part.disk == disk {
                        push(Node::Partition(index), 1);
                    }
                }
            }
        }

        for index in 0..self.layout.raids.len() {
            push(Node::Raid(index), 0);
        }

        for group in self.groups() {
            let node = Node::Group(group.clone());
            let open = !self.folded.contains(&node);

            push(node, 0);

            if open {
                for (index, volume) in self.layout.volumes.iter().enumerate() {
                    if volume.group == group {
                        push(Node::Volume(index), 1);
                    }
                }
            }
        }

        tree
    }

    pub fn node(&self) -> Option<Node> {
        self.tree().get(self.cursor).map(|entry| entry.node.clone())
    }

    /// `Some(true)` — свёрнутый узел, `Some(false)` — развёрнутый, `None` — лист.
    pub fn folded(&self, node: &Node) -> Option<bool> {
        matches!(node, Node::Disk(_) | Node::Group(_)).then(|| self.folded.contains(node))
    }

    pub fn text(&self, node: &Node, disk_size: impl Fn(&str) -> Option<u64>) -> String {
        let layout = &self.layout;

        match node {
            Node::Disk(path) => match disk_size(path) {
                Some(size) => format!("{path}  {}", crate::disks::human(size)),
                None => tr!("{path}  (disk not found)", "{path}  (диск не найден)"),
            },
            Node::Partition(index) => {
                let part = &layout.partitions[*index];

                format!(
                    "{:<20} {:>8}  {}{}",
                    layout.partition_device(*index),
                    part.size,
                    usage_text(&part.usage),
                    layout::crypt_note(part.crypt)
                )
            }
            Node::Raid(index) => {
                let raid = &layout.raids[*index];

                format!(
                    "{:<20} RAID{:<3}  {}  ← {}{}",
                    layout::raid_path(&raid.name),
                    raid.level.number(),
                    usage_text(&raid.usage),
                    layout.members(&raid.name).join(" "),
                    layout::crypt_note(raid.crypt)
                )
            }
            Node::Group(name) => format!("LVM {name}  ← {}", layout.volumes_of(name).join(" ")),
            Node::Volume(index) => {
                let volume = &layout.volumes[*index];

                format!(
                    "{:<20} {:>8}  {}{}",
                    layout::volume_path(&volume.group, &volume.name),
                    volume.size,
                    usage_text(&volume.usage),
                    layout::crypt_note(volume.crypt)
                )
            }
        }
    }

    /// Узлы, у которых не хватает связей: у родителей свёрнутых ветвей отмечено то, что внутри.
    pub fn broken(&self) -> HashSet<Node> {
        let layout = &self.layout;
        let filled = |group: &str| layout.volumes.iter().any(|volume| volume.group == group);
        let mut broken = HashSet::new();

        for (index, part) in layout.partitions.iter().enumerate() {
            let loose = match &part.usage {
                Use::Raid(name) => !layout.raids.iter().any(|raid| raid.name == *name),
                Use::Lvm(group) => !filled(group),
                _ => false,
            };

            if loose {
                broken.insert(Node::Partition(index));
                broken.insert(Node::Disk(part.disk.clone()));
            }
        }

        for (index, raid) in layout.raids.iter().enumerate() {
            let mut disks: Vec<&str> = layout
                .partitions
                .iter()
                .filter(|part| matches!(&part.usage, Use::Raid(name) if *name == raid.name))
                .map(|part| part.disk.as_str())
                .collect();
            let members = disks.len();

            disks.sort_unstable();
            disks.dedup();

            let orphan = matches!(&raid.usage, Use::Lvm(group) if !filled(group));

            if members < raid.level.minimum() || disks.len() != members || orphan {
                broken.insert(Node::Raid(index));
            }
        }

        for path in layout.early() {
            let exposed = layout
                .crypt_of(&path)
                .is_some_and(|crypt| crypt.unlock == Unlock::Keyfile && !crypt.detached);

            if exposed && let Some(node) = self.node_of(&path) {
                broken.insert(node);
            }
        }

        for group in self.groups() {
            if layout.volumes_of(&group).is_empty() || !filled(&group) {
                broken.insert(Node::Group(group));
            }
        }

        broken
    }

    pub fn key(&mut self, code: KeyCode) -> Outcome {
        if self.form.is_some() {
            self.form_key(code);
            return Outcome::Stay;
        }

        let last = self.tree().len().saturating_sub(1);

        match code {
            KeyCode::Esc | KeyCode::Char('q') => return Outcome::Close,
            KeyCode::Up => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Down => self.cursor = (self.cursor + 1).min(last),
            KeyCode::PageUp => self.cursor = self.cursor.saturating_sub(PAGE),
            KeyCode::PageDown => self.cursor = (self.cursor + PAGE).min(last),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = last,
            KeyCode::Left => self.fold(true),
            KeyCode::Right => self.fold(false),
            KeyCode::Char('-') => self.fold_all(true),
            KeyCode::Char('+') => self.fold_all(false),
            KeyCode::Enter => self.enter(),
            KeyCode::Char('n') => self.add(),
            KeyCode::Char('r') => self.add_raid(self.layout.raids.len()),
            KeyCode::Char('v') => self.add_volume(None, None),
            KeyCode::Char('e') | KeyCode::F(2) => self.edit(),
            KeyCode::Char('f') => self.cycle_filesystem(),
            KeyCode::Char('o') => self.cycle_tuning(),
            KeyCode::Char('c') => self.cycle_crypt(),
            KeyCode::Char('h') => self.toggle_header(),
            KeyCode::Char('d') | KeyCode::Delete => self.delete(),
            KeyCode::Char('u') => self.undo(),
            _ => {}
        }

        Outcome::Stay
    }

    fn node_of(&self, path: &str) -> Option<Node> {
        let layout = &self.layout;

        (0..layout.partitions.len())
            .find(|&index| layout.partition_device(index) == path)
            .map(Node::Partition)
            .or_else(|| {
                layout
                    .raids
                    .iter()
                    .position(|raid| layout::raid_path(&raid.name) == path)
                    .map(Node::Raid)
            })
            .or_else(|| {
                layout
                    .volumes
                    .iter()
                    .position(|volume| layout::volume_path(&volume.group, &volume.name) == path)
                    .map(Node::Volume)
            })
    }

    fn parent(&self, node: &Node) -> Option<Node> {
        match node {
            Node::Partition(index) => Some(Node::Disk(self.layout.partitions[*index].disk.clone())),
            Node::Volume(index) => Some(Node::Group(self.layout.volumes[*index].group.clone())),
            _ => None,
        }
    }

    fn select(&mut self, node: &Node) {
        if let Some(parent) = self.parent(node) {
            self.folded.remove(&parent);
        }

        if let Some(position) = self.tree().iter().position(|entry| entry.node == *node) {
            self.cursor = position;
        }
    }

    /// Левая стрелка сворачивает ветку или поднимается к родителю, правая разворачивает.
    fn fold(&mut self, fold: bool) {
        let Some(node) = self.node() else {
            return;
        };

        match (self.folded(&node), fold) {
            (Some(false), true) => {
                self.folded.insert(node);
            }
            (Some(true), false) => {
                self.folded.remove(&node);
            }
            (None, true) => {
                if let Some(parent) = self.parent(&node) {
                    self.select(&parent);
                }
            }
            _ => {}
        }
    }

    fn fold_all(&mut self, fold: bool) {
        let current = self.node();

        if fold {
            for disk in self.layout.disks() {
                self.folded.insert(Node::Disk(disk));
            }

            for group in self.groups() {
                self.folded.insert(Node::Group(group));
            }
        } else {
            self.folded.clear();
        }

        let tree = self.tree();
        let find = |node: &Node| tree.iter().position(|entry| entry.node == *node);

        self.cursor = current
            .and_then(|node| find(&node).or_else(|| self.parent(&node).and_then(|p| find(&p))))
            .unwrap_or(0);
    }

    fn enter(&mut self) {
        match self.node() {
            Some(node) if self.folded(&node).is_some() => {
                let fold = self.folded(&node) == Some(false);

                self.fold(fold);
            }
            Some(_) => self.edit(),
            None => self.add(),
        }
    }

    fn open(&mut self, item: Item, text: String, place: Place) {
        self.form = Some(Form {
            item,
            input: Input::with(&text),
            error: None,
            place,
        });
    }

    /// Следующий слой над устройством: том группы LVM или массив, которого ещё нет.
    fn layer_above(&self, node: &Node) -> Option<Layer> {
        match self.usage_of(node)? {
            Use::Lvm(group) => Some(Layer::Volume(group.clone())),
            Use::Raid(name) if !self.layout.raids.iter().any(|raid| raid.name == *name) => {
                Some(Layer::Raid(name.clone()))
            }
            _ => None,
        }
    }

    fn add_layer(&mut self, layer: Layer) {
        match layer {
            Layer::Volume(group) => self.add_volume(Some(group), None),
            Layer::Raid(name) => {
                let members = self.layout.members(&name).len();
                let level = if members >= 3 { 5 } else { 1 };

                self.open(
                    Item::Raid,
                    format!("name={name},level={level},point="),
                    Place::Insert(self.layout.raids.len()),
                );
            }
        }
    }

    fn add(&mut self) {
        if let Some(layer) = self.node().and_then(|node| self.layer_above(&node)) {
            return self.add_layer(layer);
        }

        match self.node() {
            Some(Node::Raid(index)) => self.add_raid(index + 1),
            Some(Node::Group(group)) => self.add_volume(Some(group), None),
            Some(Node::Volume(index)) => {
                let volume = self.layout.volumes[index].clone();
                let pool = if volume.is_pool() {
                    Some(volume.name)
                } else {
                    volume.pool
                };

                self.add_volume(Some(volume.group), pool);
            }
            node => self.add_partition(node),
        }
    }

    fn add_partition(&mut self, node: Option<Node>) {
        let (disk, at) = match node {
            Some(Node::Partition(index)) => (self.layout.partitions[index].disk.clone(), index + 1),
            Some(Node::Disk(disk)) => {
                let at = self
                    .layout
                    .partitions
                    .iter()
                    .rposition(|part| part.disk == disk)
                    .map_or(self.layout.partitions.len(), |index| index + 1);

                (disk, at)
            }
            _ => (self.free_disk(), self.layout.partitions.len()),
        };

        self.open(
            Item::Partition,
            format!("disk={disk},size=1GiB,point="),
            Place::Insert(at),
        );
    }

    fn add_raid(&mut self, at: usize) {
        let name = free_name(
            "md",
            self.layout.raids.iter().map(|raid| raid.name.as_str()),
        );

        self.open(
            Item::Raid,
            format!("name={name},level=1,point="),
            Place::Insert(at),
        );
    }

    fn add_volume(&mut self, group: Option<String>, pool: Option<String>) {
        let group = group
            .or_else(|| self.groups().into_iter().next())
            .unwrap_or_else(|| layout::DEFAULT_GROUP.into());
        let at = self
            .layout
            .volumes
            .iter()
            .rposition(|volume| volume.group == group)
            .map_or(self.layout.volumes.len(), |index| index + 1);
        let name = free_name(
            "vol",
            self.layout
                .volumes
                .iter()
                .filter(|volume| volume.group == group)
                .map(|volume| volume.name.as_str()),
        );

        let pool = pool.map_or_else(String::new, |pool| format!("pool={pool},"));

        self.open(
            Item::Volume,
            format!("vg={group},{pool}name={name},size=10GiB,point="),
            Place::Insert(at),
        );
    }

    fn free_disk(&self) -> String {
        let used = self.layout.disks();

        self.disks
            .iter()
            .find(|disk| !used.contains(disk))
            .or(self.disks.first())
            .cloned()
            .unwrap_or_else(|| "/dev/sda".into())
    }

    fn edit(&mut self) {
        match self.node() {
            Some(Node::Partition(index)) => self.open(
                Item::Partition,
                self.layout.partitions[index].to_string(),
                Place::Replace(index),
            ),
            Some(Node::Raid(index)) => self.open(
                Item::Raid,
                self.layout.raids[index].to_string(),
                Place::Replace(index),
            ),
            Some(Node::Volume(index)) => self.open(
                Item::Volume,
                self.layout.volumes[index].to_string(),
                Place::Replace(index),
            ),
            _ => {}
        }
    }

    /// Файловая система меняется только у того, что монтируется в каталог.
    fn cycle_filesystem(&mut self) {
        let Some(node) = self.node() else {
            return;
        };
        let Some(Use::Mount { path, fs, tuning }) = self.usage_of(&node) else {
            return;
        };
        let changed = Use::Mount {
            path: path.clone(),
            fs: fs.next(),
            tuning: Tuning {
                mkfs: Vec::new(),
                ..tuning.clone()
            },
        };
        let mut next = self.layout.clone();

        match node {
            Node::Partition(index) => next.partitions[index].usage = changed,
            Node::Raid(index) => next.raids[index].usage = changed,
            Node::Volume(index) => next.volumes[index].usage = changed,
            _ => return,
        }

        self.record(next);
    }

    /// Готовые настройки файловой системы по кругу: по умолчанию → SSD → большие файлы → по умолчанию.
    fn cycle_tuning(&mut self) {
        let Some(node) = self.node() else {
            return;
        };
        let Some(Use::Mount { path, fs, tuning }) = self.usage_of(&node) else {
            return;
        };
        let current = PRESETS
            .iter()
            .position(|preset| preset.tuning(*fs) == *tuning);
        let next = match current {
            None if tuning.is_default() => Some(0),
            None => None,
            Some(index) => (index + 1 < PRESETS.len()).then_some(index + 1),
        };
        let changed = Use::Mount {
            path: path.clone(),
            fs: *fs,
            tuning: next.map_or_else(Tuning::default, |index| PRESETS[index].tuning(*fs)),
        };
        let mut layout = self.layout.clone();

        match node {
            Node::Partition(index) => layout.partitions[index].usage = changed,
            Node::Raid(index) => layout.raids[index].usage = changed,
            Node::Volume(index) => layout.volumes[index].usage = changed,
            _ => return,
        }

        self.record(layout);
    }

    fn crypt_of(&self, node: &Node) -> Option<Option<Crypt>> {
        let layout = &self.layout;

        let found = match node {
            Node::Partition(index) => Some(layout.partitions[*index].crypt),
            Node::Raid(index) => Some(layout.raids[*index].crypt),
            Node::Volume(index) => Some(layout.volumes[*index].crypt),
            _ => None,
        };

        found.filter(|_| self.encryptable(node))
    }

    /// Шифруется всё, кроме разделов `/boot` и biosboot: их читают прошивка и загрузчик.
    fn encryptable(&self, node: &Node) -> bool {
        self.usage_of(node)
            .is_some_and(|usage| !matches!(usage, Use::Boot | Use::BiosBoot))
    }

    fn set_crypt(&mut self, node: &Node, crypt: Option<Crypt>) {
        let mut next = self.layout.clone();

        match node {
            Node::Partition(index) => next.partitions[*index].crypt = crypt,
            Node::Raid(index) => next.raids[*index].crypt = crypt,
            Node::Volume(index) => next.volumes[*index].crypt = crypt,
            _ => return,
        }

        self.record(next);
    }

    /// Без шифрования → пароль → ключ-файл (заголовок сразу отдельно) → без шифрования.
    fn cycle_crypt(&mut self) {
        let Some(node) = self.node() else {
            return;
        };
        let Some(current) = self.crypt_of(&node) else {
            return;
        };
        let next = match current {
            None => Some(Crypt {
                unlock: Unlock::Typed,
                detached: false,
            }),
            Some(Crypt {
                unlock: Unlock::Typed,
                ..
            }) => Some(Crypt {
                unlock: Unlock::Keyfile,
                detached: true,
            }),
            Some(_) => None,
        };

        self.set_crypt(&node, next);
    }

    fn toggle_header(&mut self) {
        let Some(node) = self.node() else {
            return;
        };

        if let Some(Some(crypt)) = self.crypt_of(&node) {
            self.set_crypt(
                &node,
                Some(Crypt {
                    detached: !crypt.detached,
                    ..crypt
                }),
            );
        }
    }

    fn usage_of(&self, node: &Node) -> Option<&Use> {
        match node {
            Node::Partition(index) => Some(&self.layout.partitions[*index].usage),
            Node::Raid(index) => Some(&self.layout.raids[*index].usage),
            Node::Volume(index) => Some(&self.layout.volumes[*index].usage),
            _ => None,
        }
    }

    /// Подсказка по клавишам для текущего места: пары «клавиша — действие».
    pub fn keys(&self) -> Vec<(&'static str, &'static str)> {
        if self.form.is_some() {
            return vec![
                ("Enter", tt!("apply", "применить")),
                (
                    "Tab",
                    tt!(
                        "purpose: /home /var /tmp …",
                        "назначение: /home /var /tmp …"
                    ),
                ),
                ("Esc", tt!("cancel", "отмена")),
                ("← →", tt!("cursor", "курсор")),
            ];
        }

        let node = self.node();
        let mut keys = vec![("↑ ↓", tt!("move", "двигаться"))];

        match &node {
            Some(Node::Disk(_)) => keys.extend([
                ("← →", tt!("collapse / expand", "свернуть / развернуть")),
                ("n", tt!("partition", "раздел")),
                ("d", tt!("delete disk", "удалить диск")),
            ]),
            Some(Node::Group(_)) => keys.extend([
                ("← →", tt!("collapse / expand", "свернуть / развернуть")),
                ("n", tt!("volume", "том")),
                ("d", tt!("delete volumes", "удалить тома")),
            ]),
            Some(item) => {
                keys.push(("e", tt!("edit", "править")));

                if matches!(self.usage_of(item), Some(Use::Mount { .. })) {
                    keys.push(("f", tt!("file system", "файловая система")));
                    keys.push(("o", tt!("FS options", "параметры ФС")));
                }

                if let Some(crypt) = self.crypt_of(item) {
                    keys.push(("c", tt!("encryption", "шифрование")));

                    if crypt.is_some() {
                        keys.push(("h", tt!("detached header", "заголовок отдельно")));
                    }
                }

                match self.layer_above(item) {
                    Some(Layer::Volume(_)) => {
                        keys.push(("n", tt!("volume in this group", "том в этой группе")))
                    }
                    Some(Layer::Raid(_)) => {
                        keys.push(("n", tt!("create this array", "создать этот массив")))
                    }
                    None if matches!(item, Node::Volume(_) | Node::Raid(_)) => {
                        keys.push(("n", tt!("add", "добавить")));
                    }
                    None => keys.push(("n", tt!("add", "добавить"))),
                }

                keys.push(("d", tt!("delete", "удалить")));
            }
            None => keys.push(("n", tt!("partition", "раздел"))),
        }

        keys.extend([("r", "RAID"), ("v", tt!("LVM volume", "том LVM"))]);
        keys.push((
            "NUM+ NUM−",
            tt!("expand / collapse all", "развернуть / свернуть всё"),
        ));

        if !self.history.is_empty() {
            keys.push(("u", tt!("undo", "отменить")));
        }

        keys.push(("Esc", tt!("done", "готово")));
        keys
    }

    fn delete(&mut self) {
        let Some(node) = self.node() else {
            return;
        };
        let mut next = self.layout.clone();

        match node {
            Node::Disk(disk) => next.partitions.retain(|part| part.disk != disk),
            Node::Partition(index) => drop(next.partitions.remove(index)),
            Node::Raid(index) => drop(next.raids.remove(index)),
            Node::Group(group) => next.volumes.retain(|volume| volume.group != group),
            Node::Volume(index) => drop(next.volumes.remove(index)),
        }

        self.record(next);
    }

    fn undo(&mut self) {
        if let Some(previous) = self.history.pop() {
            self.layout = previous;
            self.changed = true;
            self.cursor = self.cursor.min(self.tree().len().saturating_sub(1));
        }
    }

    fn record(&mut self, next: Layout) {
        if self.history.len() == HISTORY {
            self.history.remove(0);
        }

        self.history.push(std::mem::replace(&mut self.layout, next));
        self.changed = true;
        self.cursor = self.cursor.min(self.tree().len().saturating_sub(1));
    }

    fn form_key(&mut self, code: KeyCode) {
        let Some(form) = self.form.as_mut() else {
            return;
        };

        match code {
            KeyCode::Esc => self.form = None,
            KeyCode::Enter => self.submit(),
            KeyCode::Tab => self.suggest(1),
            KeyCode::BackTab => self.suggest(-1),
            KeyCode::Char(symbol) => form.input.insert(symbol),
            KeyCode::Backspace => form.input.backspace(),
            KeyCode::Delete => form.input.delete(),
            KeyCode::Left => form.input.left(),
            KeyCode::Right => form.input.right(),
            KeyCode::Home => form.input.home(),
            KeyCode::End => form.input.end(),
            _ => {}
        }
    }

    /// Назначения, которые можно подставить в `point=` для открытой формы.
    fn uses(&self, item: Item) -> Vec<String> {
        let mut uses: Vec<String> = COMMON_MOUNTS.iter().map(ToString::to_string).collect();

        uses.push("swap".into());

        if item == Item::Partition {
            uses.push("boot".into());
            uses.extend(
                self.layout
                    .raids
                    .iter()
                    .map(|raid| format!("raid:{}", raid.name)),
            );
        }

        if item != Item::Volume {
            uses.extend(
                self.groups()
                    .into_iter()
                    .map(|group| format!("lvm:{group}")),
            );
        }

        uses
    }

    /// Tab листает назначения по кругу и подставляет их на место `point=`; остальные ключи не трогаются.
    fn suggest(&mut self, step: isize) {
        let Some(item) = self.form.as_ref().map(|form| form.item) else {
            return;
        };
        let uses = self.uses(item);
        let Some(form) = self.form.as_mut() else {
            return;
        };
        let text = form.input.text().to_string();
        let Some(start) = text.rfind("point=").map(|at| at + "point=".len()) else {
            return;
        };
        let end = text[start..].find(',').map_or(text.len(), |at| start + at);
        let current = uses.iter().position(|name| *name == text[start..end]);
        let next = match current {
            Some(index) => (index as isize + step).rem_euclid(uses.len() as isize) as usize,
            None if step > 0 => 0,
            None => uses.len() - 1,
        };

        form.input = Input::with(&format!("{}{}{}", &text[..start], uses[next], &text[end..]));
        form.error = None;
    }

    fn submit(&mut self) {
        let Some(mut form) = self.form.take() else {
            return;
        };

        match self.apply(&form) {
            Ok(node) => self.select(&node),
            Err(error) => {
                form.error = Some(error.to_string());
                self.form = Some(form);
            }
        }
    }

    fn apply(&mut self, form: &Form) -> Result<Node, Error> {
        let text = form.input.text().trim();
        let mut next = self.layout.clone();
        let (index, node): (usize, fn(usize) -> Node) = match form.item {
            Item::Partition => (
                put(&mut next.partitions, Partition::parse(text)?, form.place),
                Node::Partition,
            ),
            Item::Raid => (
                put(&mut next.raids, Raid::parse(text)?, form.place),
                Node::Raid,
            ),
            Item::Volume => (
                put(&mut next.volumes, Volume::parse(text)?, form.place),
                Node::Volume,
            ),
        };

        self.record(next);

        Ok(node(index))
    }
}

/// Кладёт значение на место и возвращает его номер в списке.
fn put<T>(list: &mut Vec<T>, value: T, place: Place) -> usize {
    match place {
        Place::Insert(at) => {
            let at = at.min(list.len());

            list.insert(at, value);
            at
        }
        Place::Replace(index) => {
            list[index] = value;
            index
        }
    }
}

fn free_name<'a>(prefix: &str, taken: impl Iterator<Item = &'a str>) -> String {
    let taken: Vec<&str> = taken.collect();

    (0..)
        .map(|number| format!("{prefix}{number}"))
        .find(|name| !taken.contains(&name.as_str()))
        .unwrap_or_default()
}

fn usage_text(usage: &Use) -> String {
    match usage {
        Use::Mount { path, fs, tuning } => {
            let mut text = format!("{path} ({})", fs.name());

            if !tuning.mkfs.is_empty() {
                text.push_str(&format!("  mkfs: {}", tuning.mkfs.join(" ")));
            }

            if !tuning.options.is_empty() {
                text.push_str(&tr!(
                    "  mount: {}",
                    "  монтирование: {}",
                    tuning.options.join(",")
                ));
            }

            text
        }
        Use::Boot => "boot (FAT32)".into(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(rows: &[&str]) -> Layout {
        let rows: Vec<String> = rows.iter().map(ToString::to_string).collect();

        Layout::from_rows(&rows).unwrap()
    }

    fn pooled() -> Layout {
        layout(&[
            "partition disk=/dev/sda,size=1GiB,point=boot",
            "partition disk=/dev/sda,size=rest,point=raid:md0",
            "partition disk=/dev/sdb,size=rest,point=raid:md0",
            "raid name=md0,level=1,point=lvm:vg",
            "volume vg=vg,name=root,size=20GiB,point=/",
            "volume vg=vg,name=home,size=rest,point=/home,fs=xfs",
        ])
    }

    fn editor() -> Editor {
        Editor::new(
            pooled(),
            vec!["/dev/sda".into(), "/dev/sdb".into(), "/dev/sdc".into()],
        )
    }

    fn press(editor: &mut Editor, codes: &[KeyCode]) {
        for code in codes {
            editor.key(*code);
        }
    }

    fn type_text(editor: &mut Editor, text: &str) {
        for symbol in text.chars() {
            editor.key(KeyCode::Char(symbol));
        }
    }

    fn nodes(editor: &Editor) -> Vec<Node> {
        editor.tree().into_iter().map(|entry| entry.node).collect()
    }

    #[test]
    fn the_tree_groups_partitions_by_disk_and_volumes_by_group() {
        let tree = editor().tree();
        let shape: Vec<(Node, usize)> = tree
            .into_iter()
            .map(|entry| (entry.node, entry.depth))
            .collect();

        assert_eq!(
            shape,
            [
                (Node::Disk("/dev/sda".into()), 0),
                (Node::Partition(0), 1),
                (Node::Partition(1), 1),
                (Node::Disk("/dev/sdb".into()), 0),
                (Node::Partition(2), 1),
                (Node::Raid(0), 0),
                (Node::Group("vg".into()), 0),
                (Node::Volume(0), 1),
                (Node::Volume(1), 1),
            ]
        );
    }

    #[test]
    fn arrows_move_and_stop_at_the_edges() {
        let mut editor = editor();

        press(&mut editor, &[KeyCode::Up]);
        assert_eq!(editor.cursor, 0);

        press(&mut editor, &[KeyCode::End]);
        assert_eq!(editor.cursor, 8);

        press(&mut editor, &[KeyCode::Down]);
        assert_eq!(editor.cursor, 8);

        press(&mut editor, &[KeyCode::Home, KeyCode::Down, KeyCode::Down]);
        assert_eq!(editor.node(), Some(Node::Partition(1)));
    }

    #[test]
    fn left_and_right_fold_one_branch_and_left_on_a_leaf_goes_to_the_parent() {
        let mut editor = editor();

        press(&mut editor, &[KeyCode::Left]);
        assert_eq!(editor.folded(&Node::Disk("/dev/sda".into())), Some(true));
        assert_eq!(editor.tree().len(), 7);

        press(&mut editor, &[KeyCode::Right]);
        assert_eq!(editor.tree().len(), 9);

        press(&mut editor, &[KeyCode::Down, KeyCode::Left]);
        assert_eq!(editor.node(), Some(Node::Disk("/dev/sda".into())));
    }

    #[test]
    fn plus_and_minus_fold_every_branch_and_keep_the_cursor_visible() {
        let mut editor = editor();

        press(&mut editor, &[KeyCode::End]);
        press(&mut editor, &[KeyCode::Char('-')]);

        assert_eq!(
            nodes(&editor),
            [
                Node::Disk("/dev/sda".into()),
                Node::Disk("/dev/sdb".into()),
                Node::Raid(0),
                Node::Group("vg".into()),
            ]
        );
        assert_eq!(editor.node(), Some(Node::Group("vg".into())));

        press(&mut editor, &[KeyCode::Char('+')]);
        assert_eq!(editor.tree().len(), 9);
        assert_eq!(editor.node(), Some(Node::Group("vg".into())));
    }

    #[test]
    fn enter_on_a_branch_folds_it_and_on_a_leaf_opens_the_form() {
        let mut editor = editor();

        press(&mut editor, &[KeyCode::Enter]);
        assert_eq!(editor.tree().len(), 7);

        press(
            &mut editor,
            &[KeyCode::Enter, KeyCode::Down, KeyCode::Enter],
        );
        let form = editor.form.as_ref().unwrap();

        assert_eq!(form.item, Item::Partition);
        assert_eq!(form.input.text(), "disk=/dev/sda,size=1GiB,point=boot");
    }

    #[test]
    fn a_partition_is_added_after_the_current_one_on_its_disk() {
        let mut editor = editor();

        press(&mut editor, &[KeyCode::Down, KeyCode::Char('n')]);
        assert_eq!(
            editor.form.as_ref().unwrap().input.text(),
            "disk=/dev/sda,size=1GiB,point="
        );

        type_text(&mut editor, "swap");
        press(&mut editor, &[KeyCode::Enter]);

        assert!(editor.form.is_none());
        assert_eq!(editor.layout().partitions.len(), 4);
        assert_eq!(editor.layout().partitions[1].usage, Use::Swap);
        assert_eq!(editor.node(), Some(Node::Partition(1)));
        assert!(editor.take_changed());
        assert!(!editor.take_changed());
    }

    #[test]
    fn the_first_disk_is_suggested_for_an_empty_layout_and_n_stays_on_the_current_disk() {
        let mut editor = Editor::new(
            Layout::default(),
            vec!["/dev/sda".into(), "/dev/sdb".into()],
        );

        press(&mut editor, &[KeyCode::Char('n')]);

        assert_eq!(
            editor.form.as_ref().unwrap().input.text(),
            "disk=/dev/sda,size=1GiB,point="
        );

        type_text(&mut editor, "boot");
        press(&mut editor, &[KeyCode::Enter, KeyCode::Char('n')]);

        assert_eq!(
            editor.form.as_ref().unwrap().input.text(),
            "disk=/dev/sda,size=1GiB,point="
        );
    }

    #[test]
    fn n_on_a_group_adds_a_volume_and_r_adds_a_raid_with_a_free_name() {
        let mut editor = editor();

        press(&mut editor, &[KeyCode::End, KeyCode::Up, KeyCode::Up]);
        assert_eq!(editor.node(), Some(Node::Group("vg".into())));

        press(&mut editor, &[KeyCode::Char('n')]);
        assert_eq!(
            editor.form.as_ref().unwrap().input.text(),
            "vg=vg,name=vol0,size=10GiB,point="
        );

        press(&mut editor, &[KeyCode::Esc, KeyCode::Char('r')]);
        assert_eq!(
            editor.form.as_ref().unwrap().input.text(),
            "name=md1,level=1,point="
        );
    }

    #[test]
    fn a_bad_row_keeps_the_form_open_with_the_reason() {
        let mut editor = editor();

        press(&mut editor, &[KeyCode::Down, KeyCode::Char('n')]);
        type_text(&mut editor, "nonsense");
        press(&mut editor, &[KeyCode::Enter]);

        assert!(editor.form.as_ref().unwrap().error.is_some());
        assert_eq!(editor.layout().partitions.len(), 3);
        assert!(!editor.take_changed());

        press(&mut editor, &[KeyCode::Esc]);
        assert!(editor.form.is_none());
    }

    #[test]
    fn editing_replaces_the_element_in_place() {
        let mut editor = editor();

        press(
            &mut editor,
            &[KeyCode::Down, KeyCode::Down, KeyCode::Char('e')],
        );

        for _ in 0.."raid:md0".len() {
            press(&mut editor, &[KeyCode::Backspace]);
        }

        type_text(&mut editor, "swap");
        press(&mut editor, &[KeyCode::Enter]);

        assert_eq!(editor.layout().partitions[1].usage, Use::Swap);
        assert_eq!(editor.layout().partitions.len(), 3);
    }

    #[test]
    fn delete_removes_a_leaf_a_whole_disk_or_the_volumes_of_a_group() {
        let mut editor = editor();

        press(&mut editor, &[KeyCode::Down, KeyCode::Char('d')]);
        assert_eq!(editor.layout().partitions.len(), 2);

        press(&mut editor, &[KeyCode::Home, KeyCode::Char('d')]);
        assert_eq!(editor.layout().disks(), ["/dev/sdb"]);

        press(&mut editor, &[KeyCode::End, KeyCode::Up, KeyCode::Up]);
        assert_eq!(editor.node(), Some(Node::Group("vg".into())));

        press(&mut editor, &[KeyCode::Delete]);
        assert!(editor.layout().volumes.is_empty());
    }

    #[test]
    fn undo_brings_back_what_was_deleted() {
        let mut editor = editor();

        press(&mut editor, &[KeyCode::Home, KeyCode::Char('d')]);
        assert_eq!(editor.layout().partitions.len(), 1);

        press(&mut editor, &[KeyCode::Char('u')]);
        assert_eq!(editor.layout(), &pooled());

        press(&mut editor, &[KeyCode::Char('u')]);
        assert_eq!(editor.layout(), &pooled());
    }

    #[test]
    fn missing_links_are_marked_on_the_node_and_its_parent() {
        let mut editor = editor();

        assert!(editor.broken().is_empty());

        press(&mut editor, &[KeyCode::End, KeyCode::Up, KeyCode::Up]);
        press(&mut editor, &[KeyCode::Char('d')]);

        let broken = editor.broken();

        assert!(broken.contains(&Node::Raid(0)));
        assert!(broken.contains(&Node::Group("vg".into())));
        assert!(!broken.contains(&Node::Partition(0)));
    }

    #[test]
    fn a_raid_without_enough_members_on_different_disks_is_marked() {
        let mut editor = editor();

        press(
            &mut editor,
            &[KeyCode::Down, KeyCode::Down, KeyCode::Char('d')],
        );
        assert!(editor.broken().contains(&Node::Raid(0)));

        let same_disk = layout(&[
            "partition disk=/dev/sda,size=1GiB,point=raid:md0",
            "partition disk=/dev/sda,size=rest,point=raid:md0",
            "raid name=md0,level=1,point=/",
        ]);

        assert!(
            Editor::new(same_disk, Vec::new())
                .broken()
                .contains(&Node::Raid(0))
        );
    }

    #[test]
    fn a_partition_pointing_at_a_missing_array_marks_its_disk() {
        let editor = Editor::new(
            layout(&["partition disk=/dev/sda,size=rest,point=raid:nope"]),
            Vec::new(),
        );
        let broken = editor.broken();

        assert!(broken.contains(&Node::Partition(0)));
        assert!(broken.contains(&Node::Disk("/dev/sda".into())));
    }

    #[test]
    fn the_node_text_shows_devices_sizes_and_links() {
        let editor = editor();
        let size = |disk: &str| (disk == "/dev/sda").then_some(100 << 30);

        assert!(
            editor
                .text(&Node::Disk("/dev/sda".into()), size)
                .contains("100.0 ГиБ")
        );
        assert!(
            editor
                .text(&Node::Disk("/dev/sdb".into()), size)
                .contains("не найден")
        );
        assert!(editor.text(&Node::Partition(1), size).contains("/dev/sda2"));
        assert!(
            editor
                .text(&Node::Raid(0), size)
                .contains("← /dev/sda2 /dev/sdb1")
        );
        assert!(editor.text(&Node::Volume(1), size).contains("/home (xfs)"));
    }

    #[test]
    fn escape_and_q_close_the_editor_only_outside_the_form() {
        let mut editor = editor();

        assert_eq!(editor.key(KeyCode::Char('q')), Outcome::Close);
        assert_eq!(editor.key(KeyCode::Esc), Outcome::Close);

        editor.key(KeyCode::Char('r'));

        assert_eq!(editor.key(KeyCode::Esc), Outcome::Stay);
        assert!(editor.form.is_none());
    }

    #[test]
    fn f_switches_the_filesystem_of_a_mount_and_ignores_other_elements() {
        let mut editor = editor();

        press(&mut editor, &[KeyCode::End, KeyCode::Char('f')]);
        assert_eq!(editor.node(), Some(Node::Volume(1)));
        assert_eq!(
            editor.layout().volumes[1].usage,
            Use::Mount {
                path: "/home".into(),
                fs: layout::Fs::Btrfs,
                tuning: Tuning::default(),
            }
        );

        press(&mut editor, &[KeyCode::Char('f')]);
        assert!(matches!(
            editor.layout().volumes[1].usage,
            Use::Mount {
                fs: layout::Fs::Ext4,
                ..
            }
        ));

        press(&mut editor, &[KeyCode::Home, KeyCode::Char('f')]);
        press(&mut editor, &[KeyCode::Down, KeyCode::Char('f')]);
        assert_eq!(editor.layout().partitions[0].usage, Use::Boot);

        press(&mut editor, &[KeyCode::Char('u'), KeyCode::Char('u')]);
        assert_eq!(editor.layout(), &pooled());
    }

    #[test]
    fn the_key_hints_follow_the_place_and_the_state() {
        let mut editor = editor();
        let has = |editor: &Editor, key: &str| editor.keys().iter().any(|(name, _)| *name == key);

        assert!(has(&editor, "n") && !has(&editor, "e") && !has(&editor, "u"));

        press(&mut editor, &[KeyCode::Down]);
        assert!(has(&editor, "e") && !has(&editor, "f"));

        press(&mut editor, &[KeyCode::End]);
        assert!(has(&editor, "f") && has(&editor, "NUM+ NUM−"));

        press(&mut editor, &[KeyCode::Char('d')]);
        assert!(has(&editor, "u"));

        press(&mut editor, &[KeyCode::Char('n')]);
        assert_eq!(editor.keys()[0].0, "Enter");
        assert!(!has(&editor, "n"));
    }

    #[test]
    fn tab_fills_the_use_with_common_mounts_and_keeps_the_rest_of_the_row() {
        let mut editor = editor();

        press(
            &mut editor,
            &[KeyCode::Down, KeyCode::Char('n'), KeyCode::Tab],
        );
        assert_eq!(
            editor.form.as_ref().unwrap().input.text(),
            "disk=/dev/sda,size=1GiB,point=/"
        );

        press(&mut editor, &[KeyCode::Tab, KeyCode::Tab]);
        assert_eq!(
            editor.form.as_ref().unwrap().input.text(),
            "disk=/dev/sda,size=1GiB,point=/var"
        );

        press(
            &mut editor,
            &[KeyCode::BackTab, KeyCode::BackTab, KeyCode::BackTab],
        );
        assert!(
            editor
                .form
                .as_ref()
                .unwrap()
                .input
                .text()
                .ends_with("point=lvm:vg")
        );

        press(&mut editor, &[KeyCode::Enter]);
        assert_eq!(editor.layout().partitions[1].usage, Use::Lvm("vg".into()));
    }

    #[test]
    fn tab_keeps_a_filesystem_after_the_use() {
        let mut editor = editor();

        press(
            &mut editor,
            &[KeyCode::End, KeyCode::Char('e'), KeyCode::Tab],
        );

        let text = editor.form.as_ref().unwrap().input.text().to_string();

        assert_eq!(text, "vg=vg,name=home,size=rest,point=/var,fs=xfs");
    }

    #[test]
    fn var_and_home_can_be_separate_volumes_and_partitions() {
        let mut editor = editor();

        press(&mut editor, &[KeyCode::End, KeyCode::Char('n')]);
        press(
            &mut editor,
            &[KeyCode::Tab, KeyCode::Tab, KeyCode::Tab, KeyCode::Enter],
        );

        assert_eq!(
            editor.layout().volumes[2].usage,
            Use::Mount {
                path: "/var".into(),
                fs: layout::Fs::Ext4,
                tuning: Tuning::default(),
            }
        );
    }

    fn crypt_of_home(editor: &Editor) -> Option<Crypt> {
        editor.layout().volumes[1].crypt
    }

    #[test]
    fn c_cycles_none_pass_key_and_none_again() {
        let mut editor = editor();

        press(&mut editor, &[KeyCode::End]);
        assert_eq!(crypt_of_home(&editor), None);

        press(&mut editor, &[KeyCode::Char('c')]);
        assert_eq!(
            crypt_of_home(&editor),
            Some(Crypt {
                unlock: Unlock::Typed,
                detached: false
            })
        );

        press(&mut editor, &[KeyCode::Char('c')]);
        assert_eq!(
            crypt_of_home(&editor),
            Some(Crypt {
                unlock: Unlock::Keyfile,
                detached: true
            })
        );

        press(&mut editor, &[KeyCode::Char('c')]);
        assert_eq!(crypt_of_home(&editor), None);
    }

    #[test]
    fn h_toggles_the_detached_header_only_on_an_encrypted_element() {
        let mut editor = editor();

        press(&mut editor, &[KeyCode::End, KeyCode::Char('h')]);
        assert_eq!(crypt_of_home(&editor), None);

        press(&mut editor, &[KeyCode::Char('c'), KeyCode::Char('h')]);
        assert_eq!(
            crypt_of_home(&editor).map(|crypt| crypt.detached),
            Some(true)
        );

        press(&mut editor, &[KeyCode::Char('h')]);
        assert_eq!(
            crypt_of_home(&editor).map(|crypt| crypt.detached),
            Some(false)
        );
    }

    #[test]
    fn disks_groups_and_boot_cannot_be_encrypted_and_undo_takes_it_back() {
        let mut editor = editor();

        press(&mut editor, &[KeyCode::Home, KeyCode::Char('c')]);
        editor.cursor = editor
            .tree()
            .iter()
            .position(|entry| entry.node == Node::Partition(0))
            .unwrap();
        press(&mut editor, &[KeyCode::Char('c')]);
        assert_eq!(editor.node(), Some(Node::Partition(0)));
        assert_eq!(editor.layout(), &pooled());

        press(
            &mut editor,
            &[KeyCode::End, KeyCode::Char('c'), KeyCode::Char('u')],
        );
        assert_eq!(editor.layout(), &pooled());
    }

    #[test]
    fn encryption_is_shown_in_the_node_text_and_in_the_key_hints() {
        let mut editor = editor();
        let has = |editor: &Editor, key: &str| editor.keys().iter().any(|(name, _)| *name == key);
        let text = |editor: &Editor| editor.text(&editor.node().unwrap(), |_| None);

        press(&mut editor, &[KeyCode::End]);
        assert!(has(&editor, "c") && !has(&editor, "h"));
        assert!(!text(&editor).contains("LUKS"));

        press(&mut editor, &[KeyCode::Char('c'), KeyCode::Char('h')]);
        assert!(has(&editor, "h"));
        assert!(text(&editor).contains("[LUKS: пароль, заголовок отдельно]"));

        press(&mut editor, &[KeyCode::Home, KeyCode::Down]);
        assert!(!has(&editor, "c"));
    }

    #[test]
    fn a_key_in_the_initramfs_without_a_detached_header_is_marked() {
        let editor = Editor::new(
            layout(&[
                "partition disk=/dev/sda,size=1GiB,point=boot",
                "partition disk=/dev/sda,size=rest,point=/,crypt=key",
            ]),
            vec!["/dev/sda".into()],
        );

        assert!(editor.broken().contains(&Node::Partition(1)));

        let fixed = Editor::new(
            layout(&[
                "partition disk=/dev/sda,size=1GiB,point=boot",
                "partition disk=/dev/sda,size=rest,point=/,crypt=key,header=detached",
            ]),
            vec!["/dev/sda".into()],
        );

        assert!(fixed.broken().is_empty());
    }

    fn select_node(editor: &mut Editor, node: Node) {
        editor.cursor = editor
            .tree()
            .iter()
            .position(|entry| entry.node == node)
            .unwrap();
    }

    #[test]
    fn n_on_an_lvm_member_opens_a_volume_of_its_group() {
        let mut editor = Editor::new(
            layout(&[
                "partition disk=/dev/sda,size=1GiB,point=boot",
                "partition disk=/dev/sda,size=rest,point=lvm:vg",
            ]),
            vec!["/dev/sda".into()],
        );

        select_node(&mut editor, Node::Partition(1));
        press(&mut editor, &[KeyCode::Char('n')]);

        let form = editor.form.as_ref().unwrap();

        assert_eq!(form.item, Item::Volume);
        assert!(form.input.text().starts_with("vg=vg,name="));
    }

    #[test]
    fn n_on_a_raid_member_without_an_array_opens_that_array() {
        let mut editor = Editor::new(
            layout(&[
                "partition disk=/dev/sda,size=1GiB,point=boot",
                "partition disk=/dev/sda,size=rest,point=raid:md7",
                "partition disk=/dev/sdb,size=rest,point=raid:md7",
            ]),
            vec!["/dev/sda".into(), "/dev/sdb".into()],
        );

        select_node(&mut editor, Node::Partition(1));
        press(&mut editor, &[KeyCode::Char('n')]);

        let form = editor.form.as_ref().unwrap();

        assert_eq!(form.item, Item::Raid);
        assert_eq!(form.input.text(), "name=md7,level=1,point=");
    }

    #[test]
    fn n_on_a_raid_with_lvm_opens_a_volume_and_on_a_partition_a_partition() {
        let mut editor = editor();

        select_node(&mut editor, Node::Raid(0));
        press(&mut editor, &[KeyCode::Char('n')]);
        assert_eq!(editor.form.as_ref().unwrap().item, Item::Volume);
        press(&mut editor, &[KeyCode::Esc]);

        select_node(&mut editor, Node::Partition(0));
        press(&mut editor, &[KeyCode::Char('n')]);
        assert_eq!(editor.form.as_ref().unwrap().item, Item::Partition);
    }

    #[test]
    fn the_key_hint_names_the_layer_that_n_will_create() {
        let mut editor = editor();
        let hint = |editor: &Editor| {
            editor
                .keys()
                .iter()
                .find(|(name, _)| *name == "n")
                .map(|(_, text)| *text)
        };

        select_node(&mut editor, Node::Raid(0));
        assert_eq!(hint(&editor), Some("том в этой группе"));

        select_node(&mut editor, Node::Partition(0));
        assert_eq!(hint(&editor), Some("добавить"));
    }

    #[test]
    fn o_cycles_the_filesystem_presets_and_f_drops_the_ext4_arguments() {
        let mut editor = editor();
        let tuning = |editor: &Editor| match &editor.layout().volumes[0].usage {
            Use::Mount { tuning, .. } => tuning.clone(),
            other => panic!("не точка монтирования: {other:?}"),
        };

        select_node(&mut editor, Node::Volume(0));
        assert!(tuning(&editor).is_default());

        press(&mut editor, &[KeyCode::Char('o')]);
        assert_eq!(tuning(&editor).options, ["noatime", "discard"]);
        assert!(tuning(&editor).mkfs.is_empty());

        press(&mut editor, &[KeyCode::Char('o')]);
        assert_eq!(tuning(&editor).mkfs, ["-T", "largefile4", "-m", "1"]);

        press(&mut editor, &[KeyCode::Char('f')]);
        assert!(tuning(&editor).mkfs.is_empty());
        assert_eq!(tuning(&editor).options, ["noatime"]);

        press(&mut editor, &[KeyCode::Char('o')]);
        assert!(tuning(&editor).is_default());
    }

    #[test]
    fn o_ignores_elements_that_are_not_mount_points_and_the_text_shows_the_tuning() {
        let mut editor = editor();

        select_node(&mut editor, Node::Partition(0));
        press(&mut editor, &[KeyCode::Char('o')]);
        assert_eq!(editor.layout(), &pooled());

        select_node(&mut editor, Node::Volume(1));
        press(&mut editor, &[KeyCode::Char('o')]);

        let text = editor.text(&Node::Volume(1), |_| None);

        assert!(text.contains("монтирование: noatime,discard"));
    }

    #[test]
    fn n_on_a_group_a_volume_and_a_raid_does_what_a_does() {
        let mut editor = editor();

        for (node, item) in [
            (Node::Group("vg".into()), Item::Volume),
            (Node::Volume(0), Item::Volume),
            (Node::Raid(0), Item::Volume),
        ] {
            select_node(&mut editor, node);
            press(&mut editor, &[KeyCode::Char('n')]);
            assert_eq!(editor.form.as_ref().unwrap().item, item);
            press(&mut editor, &[KeyCode::Esc]);
        }

        let mut lone = Editor::new(
            layout(&[
                "partition disk=/dev/sda,size=1GiB,point=boot",
                "partition disk=/dev/sda,size=rest,point=/",
                "raid name=md0,level=1,point=/data",
            ]),
            vec!["/dev/sda".into()],
        );

        select_node(&mut lone, Node::Raid(0));
        press(&mut lone, &[KeyCode::Char('n')]);
        assert_eq!(lone.form.as_ref().unwrap().item, Item::Raid);
    }

    #[test]
    fn n_on_a_pool_or_a_thin_volume_opens_a_thin_volume_of_that_pool() {
        let mut editor = Editor::new(
            layout(&[
                "partition disk=/dev/sda,size=1GiB,point=boot",
                "partition disk=/dev/sda,size=rest,point=lvm:vg",
                "volume vg=vg,name=pool,size=100GiB,point=pool",
                "volume vg=vg,name=root,size=50GiB,point=/,pool=pool",
            ]),
            vec!["/dev/sda".into()],
        );

        for index in 0..2 {
            select_node(&mut editor, Node::Volume(index));
            press(&mut editor, &[KeyCode::Char('n')]);

            let form = editor.form.as_ref().unwrap();

            assert_eq!(form.item, Item::Volume);
            assert!(form.input.text().starts_with("vg=vg,pool=pool,name=vol0"));
            press(&mut editor, &[KeyCode::Esc]);
        }
    }

    #[test]
    fn english_keys_and_node_texts_have_no_russian() {
        crate::i18n::with(crate::i18n::Lang::En, || {
            let mut editor = editor();
            let mut texts = Vec::new();

            for _ in 0..10 {
                texts.extend(
                    editor
                        .keys()
                        .iter()
                        .map(|(key, action)| format!("{key} {action}")),
                );
                texts.push(editor.text(&Node::Disk("/dev/none".into()), |_| None));
                editor.key(KeyCode::Down);
            }

            texts.extend(
                [Item::Partition, Item::Raid, Item::Volume].map(|item| item.title().to_string()),
            );

            for text in texts {
                assert!(!crate::i18n::tests::cyrillic(&text), "{text}");
            }
        });
    }
}
