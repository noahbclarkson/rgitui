//! The Linux packaging metadata has to name the version it ships.
//!
//! AppImage managers and software centres take an app's version from the first
//! `<release>` in its AppStream metainfo whenever the desktop entry does not
//! carry one. The file listed only 0.1.0, so every AppImage and `.deb` since has
//! reported itself as 0.1.0.

const METAINFO: &str = include_str!("../resources/linux/com.rgitui.app.metainfo.xml");
const CHANGELOG: &str = include_str!("../../../CHANGELOG.md");

/// The value of `name="…"` inside the tag that starts `tag`.
fn attribute<'a>(tag: &'a str, name: &str) -> &'a str {
    let tag = &tag[..tag.find('>').expect("an unterminated <release> tag")];
    let start = tag
        .find(&format!("{name}=\""))
        .unwrap_or_else(|| panic!("<release> without a {name}: {tag}"))
        + name.len()
        + 2;
    let len = tag[start..].find('"').expect("an unterminated attribute");
    &tag[start..start + len]
}

/// `(version, date)` of every `<release>` in the metainfo, in document order.
fn metainfo_releases() -> Vec<(&'static str, &'static str)> {
    METAINFO
        .split("<release ")
        .skip(1)
        .map(|tag| (attribute(tag, "version"), attribute(tag, "date")))
        .collect()
}

/// `(version, date)` of every released section of the changelog, which heads
/// each one `## [x.y.z] - yyyy-mm-dd`.
fn changelog_releases() -> Vec<(&'static str, &'static str)> {
    CHANGELOG
        .lines()
        .filter_map(|line| line.strip_prefix("## ["))
        .filter_map(|heading| heading.split_once("] - "))
        .collect()
}

fn parse_version(version: &str) -> (u64, u64, u64) {
    let mut parts = version.split('.').map(|part| {
        part.parse::<u64>()
            .unwrap_or_else(|_| panic!("unparseable version {version}"))
    });
    let mut next = || parts.next().unwrap_or(0);
    (next(), next(), next())
}

#[test]
fn the_newest_release_is_the_version_being_built() {
    let releases = metainfo_releases();
    assert_eq!(
        releases.first().map(|(version, _)| *version),
        Some(env!("CARGO_PKG_VERSION")),
        "add a <release> for this version to com.rgitui.app.metainfo.xml"
    );
}

#[test]
fn every_changelog_release_is_listed_with_its_date() {
    let listed = metainfo_releases();
    for release in changelog_releases() {
        assert!(
            listed.contains(&release),
            "CHANGELOG.md has {} released on {}, but the metainfo does not",
            release.0,
            release.1
        );
    }
}

#[test]
fn releases_are_listed_newest_first() {
    let versions: Vec<_> = metainfo_releases()
        .into_iter()
        .map(|(version, _)| parse_version(version))
        .collect();
    assert!(
        versions.windows(2).all(|pair| pair[0] > pair[1]),
        "metainfo releases are out of order or repeated: {versions:?}"
    );
}
