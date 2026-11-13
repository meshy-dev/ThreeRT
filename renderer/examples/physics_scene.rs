//! The engine's world and its physics for a host that draws the scene itself: the three.js view (tools/three), which
//! has three.js for the picture and nothing for `mesh.physics`, `Raycaster.intersectPhysics` or a `Shapecaster`.
//! Built for `wasm32-unknown-unknown` as a plain module (no wasm-bindgen: tools/three/physics.ts builds and loads
//! it), it holds the objects and geometries the scene binding sends, as the renderer's world does, and the physics
//! world on them: the calls go through the code the renderer's own `__engine` runs (`api`, `physics_core::bridge`),
//! and the world is stepped at the same fixed tick, so a game moves here as it does in the renderer.
//!
//! The boundary is by pointer and length into the module's memory (`scene_alloc` / `scene_free`); text is UTF-8.
//! A call returns 0, or 1 with its error as the answer (`scene_answer` / `scene_answer_len`).
use physics_core::bridge::Bridge;
use std::alloc::Layout;
use std::cell::RefCell;
use threert_render::{api, world::World};

#[derive(Default)]
struct State {
    world: World,
    bridge: Bridge,
    /// The last call's answer (a physics call's JSON, an error's text), and the poses as last asked for.
    answer: String,
    poses: Vec<f32>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::default();
}

fn layout(len: usize) -> Layout {
    Layout::from_size_align(len.max(1), 8).expect("a layout")
}

/// `len` bytes of the module's memory for the caller to fill (aligned for numbers).
#[no_mangle]
pub extern "C" fn scene_alloc(len: usize) -> *mut u8 {
    unsafe { std::alloc::alloc(layout(len)) }
}

/// # Safety
/// `ptr` and `len` as `scene_alloc` gave and took them.
#[no_mangle]
pub unsafe extern "C" fn scene_free(ptr: *mut u8, len: usize) {
    std::alloc::dealloc(ptr, layout(len))
}

unsafe fn text<'a>(ptr: *const u8, len: usize) -> &'a str {
    std::str::from_utf8(std::slice::from_raw_parts(ptr, len)).unwrap_or("")
}

unsafe fn numbers<'a, T>(ptr: *const T, len: usize) -> &'a [T] {
    if len == 0 {
        &[]
    } else {
        std::slice::from_raw_parts(ptr, len)
    }
}

/// The call's result as its status, its answer (or its error) kept for the caller to read.
fn done(s: &mut State, r: Result<String, String>) -> i32 {
    let ok = r.is_ok();
    s.answer = r.unwrap_or_else(|e| e);
    if ok {
        0
    } else {
        1
    }
}

fn with(f: impl FnOnce(&mut State) -> Result<String, String>) -> i32 {
    STATE.with_borrow_mut(|s| {
        let r = f(s);
        done(s, r)
    })
}

#[no_mangle]
pub extern "C" fn scene_answer() -> *const u8 {
    STATE.with_borrow(|s| s.answer.as_ptr())
}

#[no_mangle]
pub extern "C" fn scene_answer_len() -> usize {
    STATE.with_borrow(|s| s.answer.len())
}

/// `__engine.geometry`: what a collider is made from (positions, three numbers a vertex, and triangle indices).
/// # Safety
/// The pointers and lengths name numbers in the module's memory.
#[no_mangle]
pub unsafe extern "C" fn scene_geometry(id: u32, positions: *const f32, np: usize, indices: *const u32, ni: usize) -> i32 {
    let (p, i) = (numbers(positions, np), numbers(indices, ni));
    // (Normals and texture coordinates are the picture's: the world wants their counts right, the physics reads neither.)
    with(|s| api::geometry(&mut s.world, id, p, &vec![0.0; p.len()], &vec![0.0; p.len() / 3 * 2], i, None, None).map(|_| String::new()).map_err(|e| e.to_string()))
}

/// `__engine.object`: the descriptor's JSON.
/// # Safety
/// As above.
#[no_mangle]
pub unsafe extern "C" fn scene_object(id: u32, desc: *const u8, len: usize) -> i32 {
    let d = text(desc, len);
    with(|s| {
        let j: serde_json::Value = serde_json::from_str(d).map_err(|e| format!("engine.object: {e}"))?;
        api::object(&mut s.world, id, &j).map(|_| String::new()).map_err(|e| e.to_string())
    })
}

/// `__engine.transform`: position, rotation (xyzw), scale.
/// # Safety
/// As above.
#[no_mangle]
pub unsafe extern "C" fn scene_transform(id: u32, trs: *const f32, len: usize) -> i32 {
    let t = numbers(trs, len);
    with(|s| api::transform(&mut s.world, id, t).map(|_| String::new()).map_err(|e| e.to_string()))
}

/// `__engine.remove`: the object, what is under it, and their bodies.
#[no_mangle]
pub extern "C" fn scene_remove(id: u32) {
    STATE.with_borrow_mut(|s| {
        let gone = s.world.remove(id);
        s.bridge.objects_removed(&gone);
    })
}

/// `__engine.physics(op, args)`: the answer is its JSON.
/// # Safety
/// As above.
#[no_mangle]
pub unsafe extern "C" fn scene_physics(op: *const u8, op_len: usize, args: *const u8, args_len: usize) -> i32 {
    let (op, args) = (text(op, op_len), text(args, args_len));
    with(|s| {
        let a: serde_json::Value = serde_json::from_str(args).map_err(|e| format!("engine.physics: {e}"))?;
        let State { world, bridge, .. } = s;
        bridge.call(world, op, &a).map(|v| v.to_string())
    })
}

/// A frame of `dt` seconds: the whole ticks that fit, and the objects of the bodies that moved placed. Returns the
/// ticks made.
#[no_mangle]
pub extern "C" fn scene_advance(dt: f64) -> u32 {
    STATE.with_borrow_mut(|s| {
        let ticks = s.bridge.due(dt);
        for _ in 0..ticks {
            s.bridge.tick();
        }
        s.bridge.sync(&mut s.world);
        // (Nobody draws this world: what changed in it is the host's to show, from the poses.)
        s.world.take_changes();
        ticks
    })
}

/// `__engine.physicsPoses()`: seven numbers a slot (position, rotation xyzw), in the space of the object's parent.
#[no_mangle]
pub extern "C" fn scene_poses() -> *const f32 {
    STATE.with_borrow_mut(|s| {
        s.poses = s.bridge.poses().to_vec();
        s.poses.as_ptr()
    })
}

#[no_mangle]
pub extern "C" fn scene_poses_len() -> usize {
    STATE.with_borrow(|s| s.poses.len())
}
