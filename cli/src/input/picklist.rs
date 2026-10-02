/* Copyright 2026 Justin Madru <justin.jdm64@gmail.com>
 * SPDX-License-Identifier: Apache-2.0
 */

pub struct PickList<T> {
    pub selected: usize,
    pub rows: Vec<T>,
    pub filtered: Vec<usize>,
}

impl<T> PickList<T> {
    pub fn new(rows: Vec<T>, filtered: Vec<usize>) -> Self {
        let mut list = Self {
            selected: 0,
            rows,
            filtered,
        };
        list.clamp_selected();
        list
    }

    pub fn all(rows: Vec<T>) -> Self {
        let filtered = (0..rows.len()).collect();
        Self::new(rows, filtered)
    }

    pub fn is_empty(&self) -> bool {
        self.filtered.is_empty()
    }

    pub fn selected_row_index(&self) -> Option<usize> {
        self.filtered.get(self.selected).copied()
    }

    pub fn selected_row(&self) -> Option<&T> {
        self.selected_row_index()
            .and_then(|index| self.rows.get(index))
    }

    pub fn selected_row_mut(&mut self) -> Option<&mut T> {
        let index = self.selected_row_index()?;
        self.rows.get_mut(index)
    }

    pub fn move_up(&mut self) {
        self.selected = wrap(self.selected as i32 - 1, self.filtered.len());
    }

    pub fn move_down(&mut self) {
        self.selected = wrap(self.selected as i32 + 1, self.filtered.len());
    }

    pub fn replace_filter(&mut self, filtered: Vec<usize>) {
        self.filtered = filtered;
        self.clamp_selected();
    }

    pub fn replace_rows(&mut self, rows: Vec<T>, filtered: Vec<usize>) {
        self.rows = rows;
        self.filtered = filtered;
        self.clamp_selected();
    }

    pub fn clamp_selected(&mut self) {
        self.selected = self.selected.min(self.filtered.len().saturating_sub(1));
    }

    pub fn visible_row_range(&self, max_visible: usize) -> (usize, usize) {
        let visible = self.rows.len().clamp(1, max_visible);
        let selected_row = self.selected_row_index().unwrap_or(0);
        let start = selected_row.saturating_add(1).saturating_sub(visible);
        let end = (start + visible).min(self.rows.len());
        (start, end)
    }
}

pub fn wrap(i: i32, n: usize) -> usize {
    if n > 0 {
        let m = n as i32;
        ((i % m + m) % m) as usize
    } else {
        0
    }
}
