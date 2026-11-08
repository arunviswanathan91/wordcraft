//! Binder operations on the item tree: finding, adding, moving, changing and deleting items.
//! Plain functions on [`Item`] lists, so they can be tested without files. Deleting moves an
//! item into the Trash folder ([`TRASH`]); only emptying the Trash removes it.

use crate::project::{Item, Kind, MAX_DEPTH, TRASH};

/// The item `id`, anywhere in `items`.
pub fn find<'a>(items: &'a [Item], id: &str) -> Option<&'a Item> {
    let mut stack: Vec<&Item> = items.iter().collect();
    while let Some(i) = stack.pop() {
        if i.id == id {
            return Some(i);
        }
        stack.extend(i.children.iter());
    }
    None
}

/// The item `id`, to change.
pub fn find_mut<'a>(items: &'a mut [Item], id: &str) -> Option<&'a mut Item> {
    for i in items.iter_mut() {
        if i.id == id {
            return Some(i);
        }
        if let Some(found) = find_mut(&mut i.children, id) {
            return Some(found);
        }
    }
    None
}

/// Take item `id` (with what it holds) out of the tree.
fn take(items: &mut Vec<Item>, id: &str) -> Option<Item> {
    if let Some(at) = items.iter().position(|i| i.id == id) {
        return Some(items.remove(at));
    }
    items.iter_mut().find_map(|i| take(&mut i.children, id))
}

/// The list of children of folder `parent` (`None`: the top level).
fn list_mut<'a>(items: &'a mut Vec<Item>, parent: Option<&str>) -> Result<&'a mut Vec<Item>, String> {
    match parent {
        None => Ok(items),
        Some(p) => {
            let folder = find_mut(items, p).ok_or_else(|| format!("no item {p:?}"))?;
            if folder.kind != Kind::Folder {
                return Err(format!("{p:?} is a section; items go in folders"));
            }
            Ok(&mut folder.children)
        }
    }
}

/// The depth of item `id` (1 at the top level).
pub fn depth(items: &[Item], id: &str) -> Option<usize> {
    let mut stack: Vec<(&Item, usize)> = items.iter().map(|i| (i, 1)).collect();
    while let Some((i, d)) = stack.pop() {
        if i.id == id {
            return Some(d);
        }
        stack.extend(i.children.iter().map(|c| (c, d + 1)));
    }
    None
}

/// How many levels `item` reaches below itself (0 for an item with nothing inside).
fn height(item: &Item) -> usize {
    let mut stack: Vec<(&Item, usize)> = vec![(item, 0)];
    let mut deepest = 0;
    while let Some((i, d)) = stack.pop() {
        deepest = deepest.max(d);
        stack.extend(i.children.iter().map(|c| (c, d + 1)));
    }
    deepest
}

/// Whether `inner` is `outer` or anywhere inside it.
fn within(items: &[Item], outer: &str, inner: &str) -> bool {
    find(items, outer).is_some_and(|o| o.id == inner || find(&o.children, inner).is_some())
}

/// Put `item` into folder `parent` (`None`: the top level) at `index` (past the end: last).
pub fn insert(items: &mut Vec<Item>, parent: Option<&str>, index: Option<usize>, item: Item) -> Result<(), String> {
    let below = parent.map_or(Some(0), |p| depth(items, p));
    let Some(below) = below else { return Err(format!("no item {:?}", parent.unwrap_or(""))) };
    if below + 1 + height(&item) > MAX_DEPTH {
        return Err(format!("folders can't nest more than {MAX_DEPTH} deep"));
    }
    let list = list_mut(items, parent)?;
    let at = index.unwrap_or(list.len()).min(list.len());
    list.insert(at, item);
    Ok(())
}

/// Move item `id` into folder `parent` (`None`: the top level) at `index` there, counted after
/// the item is taken out. A folder can't go inside itself; the Trash stays at the top level.
pub fn move_item(items: &mut Vec<Item>, id: &str, parent: Option<&str>, index: Option<usize>) -> Result<(), String> {
    if find(items, id).is_none() {
        return Err(format!("no item {id:?}"));
    }
    if id == TRASH && parent.is_some() {
        return Err("the Trash stays at the top level".into());
    }
    if let Some(p) = parent
        && within(items, id, p)
    {
        return Err(format!("{id:?} can't go inside itself"));
    }
    // Checked before taking it out, so a refused move changes nothing.
    let below = parent.map_or(Some(0), |p| depth(items, p)).ok_or_else(|| format!("no item {:?}", parent.unwrap_or("")))?;
    let item = find(items, id).cloned().ok_or_else(|| format!("no item {id:?}"))?;
    if below + 1 + height(&item) > MAX_DEPTH {
        return Err(format!("folders can't nest more than {MAX_DEPTH} deep"));
    }
    if let Some(p) = parent
        && find(items, p).is_some_and(|f| f.kind != Kind::Folder)
    {
        return Err(format!("{p:?} is a section; items go in folders"));
    }
    let item = take(items, id).ok_or_else(|| format!("no item {id:?}"))?;
    insert(items, parent, index, item)
}

/// Move item `id` into the Trash folder (made when there is none).
pub fn trash(items: &mut Vec<Item>, id: &str) -> Result<(), String> {
    if id == TRASH {
        return Err("the Trash can't be deleted; empty it instead".into());
    }
    if find(items, TRASH).is_none() {
        items.push(Item::new(TRASH.into(), Kind::Folder, "Trash".into()));
    }
    if let Some(t) = find_mut(items, TRASH) {
        t.include = false;
    }
    move_item(items, id, Some(TRASH), None)
}

/// Empty the Trash: its items go for good. Returns the ids of the sections that went (their
/// files are removed by the caller).
pub fn empty_trash(items: &mut [Item]) -> Vec<String> {
    let Some(t) = find_mut(items, TRASH) else { return Vec::new() };
    let gone = std::mem::take(&mut t.children);
    let mut sections = Vec::new();
    let mut stack: Vec<Item> = gone;
    while let Some(i) = stack.pop() {
        if i.kind == Kind::Section {
            sections.push(i.id.clone());
        }
        stack.extend(i.children);
    }
    sections
}

/// Every section in binder order (depth first), with whether compiling includes it (its own
/// flag and every folder's above it).
pub fn sections(items: &[Item]) -> Vec<(&Item, bool)> {
    fn walk<'a>(items: &'a [Item], included: bool, out: &mut Vec<(&'a Item, bool)>, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        for i in items {
            let on = included && i.include && i.id != TRASH;
            match i.kind {
                Kind::Section => out.push((i, on)),
                Kind::Folder => walk(&i.children, on, out, depth + 1),
            }
        }
    }
    let mut out = Vec::new();
    walk(items, true, &mut out, 1);
    out
}
