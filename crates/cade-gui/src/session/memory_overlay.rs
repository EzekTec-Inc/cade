//! Memory-viewer overlay state for [`super::SessionState`].

use super::*;

impl SessionState {
    /// Open the memory overlay. Marks the panel as loading and clears any
    /// previous error; caller is responsible for spawning the fetch.
    pub fn open_memory_overlay(&mut self) {
        if let Self::Connected {
            memory_open,
            memory_loading,
            memory_error,
            memory_save_notice,
            ..
        } = self
        {
            *memory_open = true;
            *memory_loading = true;
            *memory_error = None;
            *memory_save_notice = None;
        }
    }

    /// Close the memory overlay.  Does not clear blocks (so reopening is
    /// instant) but does reset the edit buffer + error.
    pub fn close_memory_overlay(&mut self) {
        if let Self::Connected {
            memory_open,
            memory_saving,
            memory_error,
            memory_save_notice,
            ..
        } = self
        {
            *memory_open = false;
            *memory_saving = false;
            *memory_error = None;
            *memory_save_notice = None;
        }
    }

    /// Whether the memory overlay is currently open.
    pub fn is_memory_open(&self) -> bool {
        matches!(
            self,
            Self::Connected {
                memory_open: true,
                ..
            }
        )
    }

    /// Feed the result of a successful memory fetch.  Resets selection
    /// to 0 and seeds the edit buffer with the first block.
    pub fn on_memory_loaded(&mut self, blocks: Vec<crate::api::MemoryBlock>) {
        if let Self::Connected {
            memory_blocks,
            memory_selection,
            memory_edit_buffer,
            memory_loading,
            memory_error,
            ..
        } = self
        {
            *memory_loading = false;
            *memory_error = None;
            *memory_selection = 0;
            *memory_edit_buffer = blocks.first().map(|b| b.value.clone()).unwrap_or_default();
            *memory_blocks = blocks;
        }
    }

    /// Feed an error from the memory fetch.  Clears the loading flag.
    pub fn on_memory_error(&mut self, err: &str) {
        if let Self::Connected {
            memory_loading,
            memory_saving,
            memory_error,
            memory_save_notice,
            ..
        } = self
        {
            *memory_loading = false;
            *memory_saving = false;
            *memory_error = Some(err.to_string());
            *memory_save_notice = None;
        }
    }

    /// Change which memory block is currently highlighted.  Seeds the
    /// edit buffer with the new block's value (discarding unsaved edits).
    /// Returns `true` if the selection changed, `false` otherwise.
    pub fn select_memory_block(&mut self, idx: usize) -> bool {
        if let Self::Connected {
            memory_blocks,
            memory_selection,
            memory_edit_buffer,
            memory_save_notice,
            ..
        } = self
        {
            if idx >= memory_blocks.len() {
                return false;
            }
            if *memory_selection == idx {
                return false;
            }
            *memory_selection = idx;
            *memory_edit_buffer = memory_blocks[idx].value.clone();
            *memory_save_notice = None;
            true
        } else {
            false
        }
    }

    /// Replace the edit-buffer contents — called on every TextEdit change.
    pub fn set_memory_edit_buffer(&mut self, value: &str) {
        if let Self::Connected {
            memory_edit_buffer, ..
        } = self
        {
            *memory_edit_buffer = value.to_string();
        }
    }

    /// Mark a save request as in-flight.
    pub fn on_memory_save_start(&mut self) {
        if let Self::Connected {
            memory_saving,
            memory_error,
            memory_save_notice,
            ..
        } = self
        {
            *memory_saving = true;
            *memory_error = None;
            *memory_save_notice = None;
        }
    }

    /// On successful save, persist the edit buffer into the corresponding
    /// block so the sidebar list reflects the new value, and set a
    /// transient success notice for the overlay (e.g. "Saved /project").
    pub fn on_memory_save_ok(&mut self) {
        if let Self::Connected {
            memory_blocks,
            memory_selection,
            memory_edit_buffer,
            memory_saving,
            memory_error,
            memory_save_notice,
            ..
        } = self
        {
            *memory_saving = false;
            *memory_error = None;
            if let Some(b) = memory_blocks.get_mut(*memory_selection) {
                b.value = memory_edit_buffer.clone();
                *memory_save_notice = Some(format!("Saved /{}", b.label));
            }
        }
    }

    /// Extract the `(label, value)` tuple currently being edited, so the
    /// spawn-helper can issue the PUT.  Returns `None` when the overlay
    /// is closed or no block is selected.
    pub fn memory_selected_label_value(&self) -> Option<(String, String)> {
        if let Self::Connected {
            memory_open: true,
            memory_blocks,
            memory_selection,
            memory_edit_buffer,
            ..
        } = self
        {
            memory_blocks
                .get(*memory_selection)
                .map(|b| (b.label.clone(), memory_edit_buffer.clone()))
        } else {
            None
        }
    }

    /// Whether the in-memory edit buffer differs from the currently-
    /// selected block's saved value.  Used to enable/disable the Save
    /// button and show a dirty indicator.  Returns `false` when the
    /// overlay is closed, no block is selected, or buffer == saved value.
    pub fn is_memory_dirty(&self) -> bool {
        if let Self::Connected {
            memory_open: true,
            memory_blocks,
            memory_selection,
            memory_edit_buffer,
            ..
        } = self
        {
            match memory_blocks.get(*memory_selection) {
                Some(b) => b.value != *memory_edit_buffer,
                None => false,
            }
        } else {
            false
        }
    }

    /// Transient success notice shown after a successful save.  Returns
    /// `None` when no save has completed since the last open/select/error.
    pub fn memory_save_notice(&self) -> Option<&str> {
        if let Self::Connected {
            memory_save_notice: Some(n),
            ..
        } = self
        {
            Some(n.as_str())
        } else {
            None
        }
    }
}
