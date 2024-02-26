use std::path::PathBuf;

pub(crate) trait Repr<'a> {
    fn repr(&self) -> String;
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
