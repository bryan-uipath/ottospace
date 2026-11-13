//! Reads a window's accessibility tree into plain data. Read-only: it performs no action.
//!
//! Every AX call is synchronous IPC to the app's main thread. Each element gets a short messaging
//! timeout, and the first call that times out ends the walk, so a busy app costs one timeout.

use std::collections::VecDeque;
use std::ptr::NonNull;
use std::time::{Duration, Instant};

use objc2_application_services::{AXCopyMultipleAttributeOptions, AXError, AXUIElement, AXValue, AXValueType};
use objc2_core_foundation::{CFArray, CFBoolean, CFNumber, CFRetained, CFString, CFType, CGPoint, CGSize};
use ottospace_core::ax::{Frame, Limits, MAX_TEXT, Node, Tree, Truncated, truncate};

use crate::ax::Window;

/// Per-call wait for the app, in seconds. AX's default is 6s.
const TIMEOUT: f32 = 0.5;

/// Read per node, in this order.
const ATTRS: [&str; 11] = [
    "AXRole",
    "AXSubrole",
    "AXTitle",
    "AXDescription",
    "AXValue",
    "AXPosition",
    "AXSize",
    "AXEnabled",
    "AXChildren",
    "AXIdentifier",
    "AXFocused",
];

/// How attributes are fetched. `Batched` is one IPC per node for all of them (plus one for the
/// actions); `Single` is one IPC per attribute. Both are kept to measure the difference.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fetch {
    Batched,
    Single,
}

enum Failed {
    /// The app did not answer within `TIMEOUT`.
    Busy,
    /// The element no longer exists, e.g. a table row replaced during the walk.
    Gone,
}

type Values = Vec<Option<CFRetained<CFType>>>;

/// Walks `window`'s tree within `limits`. Breadth first, so the node limit drops the deepest
/// nodes (e.g. table cells), not a toolbar that comes after a long table.
pub fn read(window: &Window, limits: Limits, fetch: Fetch) -> Tree {
    let names: Vec<CFRetained<CFString>> = ATTRS.iter().map(|a| CFString::from_str(a)).collect();
    let refs: Vec<&CFString> = names.iter().map(|n| &**n).collect();
    let names = CFArray::from_objects(&refs);
    let mut truncated = Truncated::default();
    // Each node with its parent's index. Children always come after their parent.
    let mut nodes: Vec<(Node, Option<usize>)> = Vec::new();
    let mut queue = VecDeque::from([(window.element(), None, 0)]);
    let deadline = limits.millis.map(|ms| Instant::now() + Duration::from_millis(ms));
    while let Some((element, parent, depth)) = queue.pop_front() {
        if nodes.len() >= limits.nodes {
            truncated.nodes = true;
            break;
        }
        if deadline.is_some_and(|at| Instant::now() >= at) {
            truncated.timeout = true;
            break;
        }
        // No call waits past the walk's time: one slow node cannot run it long.
        let left = deadline.map_or(TIMEOUT, |at| at.saturating_duration_since(Instant::now()).as_secs_f32());
        let (node, children) = match read_node(&element, &names, fetch, left.min(TIMEOUT)) {
            Ok(read) => read,
            Err(Failed::Gone) => continue,
            Err(Failed::Busy) => {
                truncated.timeout = true;
                break;
            }
        };
        if children.is_empty() || depth < limits.depth {
            let at = nodes.len();
            queue.extend(children.into_iter().map(|child| (child, Some(at), depth + 1)));
        } else {
            truncated.depth = true;
        }
        nodes.push((node, parent));
    }
    let count = nodes.len();
    // Fold children into parents from the back; each node's children arrive last to first.
    while nodes.len() > 1 {
        let Some((mut node, Some(parent))) = nodes.pop() else { break };
        node.children.reverse();
        nodes[parent].0.children.push(node);
    }
    let mut root = nodes.pop().map(|(node, _)| node).unwrap_or_default();
    root.children.reverse();
    Tree { root, count, truncated }
}

/// One element as plain data, and its children's handles: for a press. `None`: it is gone, or
/// the app did not answer in time.
pub(crate) fn node_of(element: &AXUIElement) -> Option<(Node, Vec<CFRetained<AXUIElement>>)> {
    let names: Vec<CFRetained<CFString>> = ATTRS.iter().map(|a| CFString::from_str(a)).collect();
    let refs: Vec<&CFString> = names.iter().map(|n| &**n).collect();
    read_node(element, &CFArray::from_objects(&refs), Fetch::Batched, TIMEOUT).ok()
}

fn read_node(
    element: &AXUIElement,
    names: &CFArray<CFString>,
    fetch: Fetch,
    timeout: f32,
) -> Result<(Node, Vec<CFRetained<AXUIElement>>), Failed> {
    // Per element: a timeout set on one element does not carry to the elements it returns.
    unsafe { element.set_messaging_timeout(timeout) };
    let values = match fetch {
        Fetch::Batched => batched(element, names)?,
        Fetch::Single => ATTRS.iter().map(|name| attr(element, name)).collect::<Result<_, _>>()?,
    };
    let text = |i: usize| string(values[i].as_deref()).filter(|s| !s.is_empty());
    let role = text(0).unwrap_or_default();
    let node = Node {
        role: role.clone(),
        subrole: text(1),
        title: text(2),
        description: text(3),
        value: text(4),
        frame: frame(values[5].as_deref(), values[6].as_deref()),
        enabled: values[7].as_deref().and_then(|v| v.downcast_ref::<CFBoolean>()).map(CFBoolean::as_bool),
        actions: actions(element)?,
        identifier: text(9),
        focused: flag(values[10].as_deref()),
        settable: INPUT.contains(&role.as_str()) && settable(element),
        children: Vec::new(),
    };
    Ok((node, children(values[8].as_deref())))
}

/// Takes a returned CF object, or says why there is none. `Ok(None)`: the element lacks it.
fn owned<T: ?Sized + objc2_core_foundation::Type>(
    err: AXError,
    out: *const T,
) -> Result<Option<CFRetained<T>>, Failed> {
    match err {
        AXError::Success => Ok(NonNull::new(out.cast_mut()).map(|ptr| unsafe { CFRetained::from_raw(ptr) })),
        AXError::CannotComplete => Err(Failed::Busy),
        AXError::InvalidUIElement => Err(Failed::Gone),
        _ => Ok(None),
    }
}

fn batched(element: &AXUIElement, names: &CFArray<CFString>) -> Result<Values, Failed> {
    let mut out: *const CFArray = std::ptr::null();
    let options = AXCopyMultipleAttributeOptions::empty();
    let err = unsafe { element.copy_multiple_attribute_values(names.as_opaque(), options, NonNull::from(&mut out)) };
    let Some(array) = owned(err, out)? else { return Ok(vec![None; ATTRS.len()]) };
    let array = unsafe { CFRetained::cast_unchecked::<CFArray<CFType>>(array) };
    // A missing attribute comes back in its slot as an AXValue holding the error; a timeout there
    // ends the walk like any other.
    let mut values = Vec::with_capacity(ATTRS.len());
    for i in 0..ATTRS.len() {
        let value = array.get(i);
        match value.as_deref().and_then(|v| v.downcast_ref::<AXValue>()).and_then(embedded_error) {
            Some(err) => owned::<CFType>(err, std::ptr::null()).map(|_| values.push(None))?,
            None => values.push(value),
        }
    }
    Ok(values)
}

/// The error an AXValue holds, if it holds one.
fn embedded_error(value: &AXValue) -> Option<AXError> {
    let mut err = AXError::Success;
    let ok = unsafe {
        value.r#type() == AXValueType::AXError && value.value(AXValueType::AXError, NonNull::from(&mut err).cast())
    };
    ok.then_some(err)
}

fn attr(element: &AXUIElement, name: &str) -> Result<Option<CFRetained<CFType>>, Failed> {
    let name = CFString::from_str(name);
    let mut out: *const CFType = std::ptr::null();
    let err = unsafe { element.copy_attribute_value(&name, NonNull::from(&mut out)) };
    owned(err, out)
}

/// Roles whose value may be settable: asked only of these, so other nodes cost no extra call.
const INPUT: [&str; 5] = ["AXTextField", "AXTextArea", "AXComboBox", "AXSearchField", "AXSlider"];

fn flag(value: Option<&CFType>) -> bool {
    value.and_then(|v| v.downcast_ref::<CFBoolean>()).is_some_and(CFBoolean::as_bool)
}

/// Whether the node's value can be set. Only asked, never set. Any failure: no, and the walk goes on.
fn settable(element: &AXUIElement) -> bool {
    let name = CFString::from_str("AXValue");
    let mut settable: u8 = 0;
    let err = unsafe { element.is_attribute_settable(&name, NonNull::from(&mut settable).cast()) };
    err == AXError::Success && settable != 0
}

fn actions(element: &AXUIElement) -> Result<Vec<String>, Failed> {
    let mut out: *const CFArray = std::ptr::null();
    let err = unsafe { element.copy_action_names(NonNull::from(&mut out)) };
    let Some(array) = owned(err, out)? else { return Ok(Vec::new()) };
    let array = unsafe { CFRetained::cast_unchecked::<CFArray<CFType>>(array) };
    Ok(array.iter().filter_map(|a| a.downcast_ref::<CFString>().map(|s| s.to_string())).collect())
}

/// Strings as they are; numbers (sliders, checkboxes) as text. Anything else is left out.
fn string(value: Option<&CFType>) -> Option<String> {
    let value = value?;
    if let Some(s) = value.downcast_ref::<CFString>() {
        return Some(truncate(&s.to_string(), MAX_TEXT));
    }
    let n = value.downcast_ref::<CFNumber>()?;
    n.as_i64().map(|i| i.to_string()).or_else(|| n.as_f64().map(|f| f.to_string()))
}

fn frame(position: Option<&CFType>, size: Option<&CFType>) -> Option<Frame> {
    let mut origin = CGPoint::ZERO;
    let mut extent = CGSize::ZERO;
    let position = position?.downcast_ref::<AXValue>()?;
    let size = size?.downcast_ref::<AXValue>()?;
    let ok = unsafe {
        position.value(AXValueType::CGPoint, NonNull::from(&mut origin).cast())
            && size.value(AXValueType::CGSize, NonNull::from(&mut extent).cast())
    };
    ok.then_some(Frame { x: origin.x, y: origin.y, w: extent.width, h: extent.height })
}

fn children(value: Option<&CFType>) -> Vec<CFRetained<AXUIElement>> {
    let Some(array) = value.and_then(|v| v.downcast_ref::<CFArray>()) else { return Vec::new() };
    let array = unsafe { &*(array as *const CFArray).cast::<CFArray<CFType>>() };
    array.iter().filter_map(|c| c.downcast::<AXUIElement>().ok()).collect()
}
