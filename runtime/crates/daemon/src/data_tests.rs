use super::*;

#[test]
fn serving_aceita_ready_e_syncing_e_recusa_o_resto() {
    assert!(serving(CollectionState::Ready));
    assert!(serving(CollectionState::Syncing));
    assert!(!serving(CollectionState::Opening));
    assert!(!serving(CollectionState::Moving));
    assert!(!serving(CollectionState::Paused));
    assert!(!serving(CollectionState::Unavailable));
    assert!(!serving(CollectionState::Failed));
}

// Do PR #1 (fix/host-lock-sem-flock): pasta de estado por coleção.
#[test]
fn folder_state_dir_is_the_default_without_the_variable() {
    assert_eq!(folder_state_dir_in(None, Path::new("/srv/vault")), None);
}

#[test]
fn folder_state_dir_gives_each_folder_its_own_subdir() {
    let base = std::ffi::OsString::from("/home/u/.local/state/mdbase-folders");
    let a = folder_state_dir_in(
        Some(base.clone()),
        Path::new("/storage/emulated/0/BIBLIOTECA"),
    )
    .unwrap();
    let b = folder_state_dir_in(Some(base), Path::new("/storage/emulated/0/OUTRA")).unwrap();
    assert!(a.starts_with("/home/u/.local/state/mdbase-folders"));
    assert_eq!(a.parent(), b.parent());
    assert_ne!(a, b);
    // One plain name under the base, never a path that leaves it.
    assert_eq!(a.components().count(), b.components().count());
    assert!(!a.file_name().unwrap().to_string_lossy().contains('/'));
}
