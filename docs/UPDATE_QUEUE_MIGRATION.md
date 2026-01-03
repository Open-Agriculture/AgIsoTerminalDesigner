# UpdateQueue Migration Guide

## Overview

The UpdateQueue system provides a better way to update objects in the pool compared to the previous clone-and-replace pattern. This document explains how to migrate existing code to use the new system.

## Why UpdateQueue?

### Problems with the Old Pattern

```rust
// OLD WAY - Problems:
// 1. Borrows mut_pool for entire duration of UI code
// 2. Clones entire pool for undo history
// 3. Difficult to have multiple UI elements updating same object
// 4. Race conditions possible with RefCell borrowing

let mut mut_pool = pool.get_mut_pool().borrow_mut();
if let Some(obj) = mut_pool.object_mut_by_id(id) {
    match obj {
        Object::WorkingSet(ws) => {
            ws.background_colour = new_value;
            ws.selectable = new_selectable;
        }
        _ => {}
    }
}
// Pool is borrowed until this scope ends!
```

### Benefits of the New Pattern

```rust
// NEW WAY - Benefits:
// 1. No long-lived borrows - queue and release immediately
// 2. Only changed values recorded for undo
// 3. Multiple UI elements can queue updates safely
// 4. All updates applied atomically at frame end

pool.queue_update(object_id, |obj| {
    if let Object::WorkingSet(ws) = obj {
        ws.background_colour = new_value;
        ws.selectable = new_selectable;
    }
});
// No borrow held - safe to queue more updates!
```

## Migration Patterns

### Pattern 1: Simple Field Update

**Before:**
```rust
fn render_parameters(&mut self, ui: &mut egui::Ui, design: &EditorProject) {
    ui.add(
        egui::Slider::new(&mut self.background_colour, 0..=255)
            .text("Background Colour")
    );
}
```

**After (keeping direct mutation for now):**
```rust
fn render_parameters(&mut self, ui: &mut egui::Ui, design: &EditorProject) {
    let object_id = self.id;
    let mut bg_colour = self.background_colour;
    
    if ui.add(
        egui::Slider::new(&mut bg_colour, 0..=255)
            .text("Background Colour")
    ).changed() {
        design.queue_update(object_id, move |obj| {
            if let Object::WorkingSet(ws) = obj {
                ws.background_colour = bg_colour;
            }
        });
    }
}
```

### Pattern 2: Multiple Fields

**Before:**
```rust
ui.checkbox(&mut self.selectable, "Selectable");
ui.checkbox(&mut self.enabled, "Enabled");
```

**After:**
```rust
let object_id = self.id;
let mut selectable = self.selectable;
let mut enabled = self.enabled;

if ui.checkbox(&mut selectable, "Selectable").changed() {
    design.queue_update(object_id, move |obj| {
        if let Object::WorkingSet(ws) = obj {
            ws.selectable = selectable;
        }
    });
}

if ui.checkbox(&mut enabled, "Enabled").changed() {
    design.queue_update(object_id, move |obj| {
        if let Object::WorkingSet(ws) = obj {
            ws.enabled = enabled;
        }
    });
}
```

### Pattern 3: Nested Object References

**Before:**
```rust
fn render_object_references_list(..., object_refs: &mut Vec<ObjectRef>, ...) {
    // Directly mutates object_refs
    object_refs.push(ObjectRef { ... });
}
```

**After:**
```rust
fn render_object_references_list(..., object_refs: &Vec<ObjectRef>, ...) {
    // Create a copy to work with
    let mut new_refs = object_refs.clone();
    
    // Work with new_refs in UI
    if /* user adds reference */ {
        new_refs.push(ObjectRef { ... });
        
        // Queue the update
        design.queue_update(object_id, move |obj| {
            if let Object::WorkingSet(ws) = obj {
                ws.object_refs = new_refs;
            }
        });
    }
}
```

## Migration Strategy

We recommend a gradual migration approach:

### Phase 1: Infrastructure (✅ Complete)
- [x] Create UpdateQueue system
- [x] Add queue_update() to EditorProject
- [x] Integrate with update_pool()

### Phase 2: Parallel Mode (Current)
- [ ] Keep existing get_mut_pool() working
- [ ] Add queue_update() calls alongside direct mutations
- [ ] Test both paths work correctly
- [ ] Document migration patterns

### Phase 3: Gradual Migration
- [ ] Convert simple objects first (NumberVariable, StringVariable)
- [ ] Then convert complex objects (WorkingSet, DataMask)
- [ ] Update render_object_references_list to use queued updates
- [ ] Update all render_parameters implementations

### Phase 4: Cleanup
- [ ] Remove get_mut_pool() method
- [ ] Remove mut_pool field
- [ ] Simplify EditorProject structure

## Testing the Migration

For each migrated component:

1. **Test Basic Update**: Verify field changes work
2. **Test Undo/Redo**: Verify undo history is created correctly  
3. **Test Multiple Updates**: Queue several updates in one frame
4. **Test Error Handling**: Try updating non-existent objects

## Example: Complete NumberVariable Migration

```rust
impl ConfigurableObject for NumberVariable {
    fn render_parameters(&mut self, ui: &mut egui::Ui, design: &EditorProject) {
        render_object_id(ui, &mut self.id, design);
        
        let object_id = self.id;
        let mut value = self.value;

        ui.horizontal(|ui| {
            ui.label("Initial Value:");
            if ui.add(egui::DragValue::new(&mut value).speed(1.0)).changed() {
                design.queue_update(object_id, move |obj| {
                    if let Object::NumberVariable(nv) = obj {
                        nv.value = value;
                    }
                });
            }
        });
    }
}
```

## Common Pitfalls

### ❌ Don't: Hold mut_pool borrow while queuing updates
```rust
// WRONG - This defeats the purpose!
let mut pool = design.get_mut_pool().borrow_mut();
design.queue_update(id, |obj| { ... }); // Will panic - pool already borrowed!
```

### ✅ Do: Queue updates without any borrow
```rust
// CORRECT
design.queue_update(id, |obj| {
    // Update object
});
// Updates will be applied later by update_pool()
```

### ❌ Don't: Capture `&mut self` in closure
```rust
// WRONG - Can't move mutable reference into closure
design.queue_update(self.id, |obj| {
    self.some_field = 5; // Error!
});
```

### ✅ Do: Capture values, not references
```rust
// CORRECT
let new_value = 5;
design.queue_update(self.id, move |obj| {
    if let Object::MyType(mt) = obj {
        mt.some_field = new_value;
    }
});
```

## Performance Characteristics

- **Memory**: O(number of changed fields) vs O(entire pool size)
- **Time**: O(number of updates) vs O(entire pool clone)
- **Undo History**: ~1KB per frame vs ~100KB+ per frame

For a typical frame with 5 field changes:
- Old system: Clone 1000+ objects = ~100KB
- New system: Queue 5 updates = ~1KB

This results in:
- 100x smaller undo history
- Faster undo/redo operations
- More undo levels can be kept in memory
