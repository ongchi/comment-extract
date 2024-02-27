use std::path::PathBuf;

use rustdoc_types::{Id, ItemEnum};

use crate::segment::ItemRef;

pub(crate) trait ItemId {
    fn id(&self) -> &Id;
}

pub(crate) trait Repr<'a> {
    fn repr(&self, root: &'a ItemRef) -> String;
}

pub(crate) trait Name {
    fn name(&self) -> &str;
}

pub(crate) trait ModulePath {
    fn path(&self) -> PathBuf;
}

pub(crate) trait RelativeTo<'a, T> {
    fn relative_to(&'a self, other: &T) -> PathBuf;
}

pub(crate) trait CrossRef<T> {
    fn cross_ref(&self, _to: &T) -> String;

    fn cross_ref_md(&self, _to: &T) -> String;
}

impl<'a, T> CrossRef<T> for &'a T
where
    T: Name + ModulePath + RelativeTo<'a, T>,
{
    fn cross_ref(&self, to: &T) -> String {
        self.relative_to(to)
            .join(format!("{}.md", to.name()))
            .to_str()
            .unwrap()
            .to_string()
    }

    fn cross_ref_md(&self, to: &T) -> String {
        format!("[{}]({})", to.name(), self.cross_ref(to))
    }
}

pub(crate) trait ExternalLink {
    fn external_link(&self, root: &ItemRef) -> String;
}

impl<T> ExternalLink for T
where
    T: ItemId + std::fmt::Debug,
{
    fn external_link(&self, root: &ItemRef) -> String {
        let crate_ = root.pool.get(root.pkg).unwrap();

        if let Some(item) = crate_.index.get(self.id()) {
            match item.crate_id {
                0 => {
                    let path = crate_
                        .paths
                        .get(&item.id)
                        .map(|s| {
                            s.path
                                .iter()
                                .rev()
                                .skip(1)
                                .rev()
                                .map(|s| s.as_ref())
                                .collect::<Vec<&str>>()
                                .join("/")
                        })
                        .unwrap();

                    format!(
                        "https://docs.rs/{}/{}/{}/{}.{}.html",
                        root.pkg,
                        crate_.crate_version.as_deref().unwrap(),
                        path,
                        match &item.inner {
                            ItemEnum::Struct(_) => "struct",
                            ItemEnum::Trait(_) => "trait",
                            others => unimplemented!("Unimplemented type: {:?}", others),
                        },
                        item.name.as_deref().unwrap()
                    )
                }
                _ => {
                    let root_url = crate_
                        .external_crates
                        .get(&item.crate_id)
                        .and_then(|c| c.html_root_url.as_deref());
                    match root_url {
                        Some(url) => url.to_string(),
                        None => "".to_string(),
                    }
                }
            }
        } else {
            "".to_string()
        }
    }
}
