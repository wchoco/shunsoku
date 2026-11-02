//! ごみ箱の方針: 上限、超えたら古いものから捨てる、上限 0 なら閉じたらすぐに破棄する、復元の順番
//! （最後に閉じたものから）、一覧。保持期間は設けない。
//!
//! 預かるもの `T` の中身は問わない。Win32 層の `trash_store` が、閉じる直前の `ScrapModel` と、一時ファイルに
//! 退避した画素をまとめて預ける。捨てたもの（`Pushed::discarded`、`set_limit` と `clear` の戻り値）は
//! 呼び出し側に返すので、呼び出し側がその一時ファイルを消す。

/// 閉じたものを預かるごみ箱。
#[derive(Debug, Clone)]
pub struct Trash<T> {
    /// 古いものから並べる。
    entries: Vec<(TrashId, T)>,
    limit: usize,
    next_id: u64,
}

/// `Trash::push` の結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pushed<T> {
    /// 入れたものの番号。上限が 0 で入れなかったときは None。
    pub id: Option<TrashId>,
    /// 上限を超えたので捨てたもの（古いものから）。
    pub discarded: Vec<T>,
}

/// ごみ箱に入れたものの番号。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TrashId(u64);

impl<T> Trash<T> {
    /// 空のごみ箱。`limit` は上限（件）。
    pub fn new(limit: usize) -> Trash<T> {
        Trash {
            entries: Vec::new(),
            limit,
            next_id: 0,
        }
    }

    /// 閉じたものを入れる。上限を超えたら古いものから捨てる。上限が 0 なら入れずに、そのまま捨てる
    /// （`Pushed::id` が None で、`discarded` に `item` が入る）。
    pub fn push(&mut self, item: T) -> Pushed<T> {
        if self.limit == 0 {
            return Pushed {
                id: None,
                discarded: vec![item],
            };
        }
        let id = TrashId(self.next_id);
        self.next_id += 1;
        self.entries.push((id, item));
        Pushed {
            id: Some(id),
            discarded: self.truncate(),
        }
    }

    /// 上限を変える。下げて超えた分は古いものから捨てて返す。
    pub fn set_limit(&mut self, limit: usize) -> Vec<T> {
        self.limit = limit;
        self.truncate()
    }

    /// 上限を超えた分を、古いものから捨てて返す。
    fn truncate(&mut self) -> Vec<T> {
        let over = self.entries.len().saturating_sub(self.limit);
        self.entries.drain(..over).map(|(_, item)| item).collect()
    }

    /// 新しいものから並べた一覧。
    pub fn list(&self) -> impl Iterator<Item = (TrashId, &T)> {
        self.entries.iter().rev().map(|(id, item)| (*id, item))
    }

    /// 最後に入れたもの（「最後に閉じたものを復元」）を取り出す。
    pub fn restore_last(&mut self) -> Option<T> {
        self.entries.pop().map(|(_, item)| item)
    }

    /// `id` のものを取り出す。もうなければ（捨てた・取り出した）None。
    pub fn restore(&mut self, id: TrashId) -> Option<T> {
        let index = self.entries.iter().position(|(i, _)| *i == id)?;
        Some(self.entries.remove(index).1)
    }

    /// `id` のもの。もうなければ None。
    pub fn get_mut(&mut self, id: TrashId) -> Option<&mut T> {
        self.entries
            .iter_mut()
            .find(|(i, _)| *i == id)
            .map(|(_, item)| item)
    }

    /// 空にして、入っていたものを古いものから返す（終了時）。
    pub fn clear(&mut self) -> Vec<T> {
        self.entries.drain(..).map(|(_, item)| item).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids<T: Copy>(trash: &Trash<T>) -> Vec<T> {
        trash.list().map(|(_, &item)| item).collect()
    }

    #[test]
    fn closed_items_are_listed_and_restored_from_the_last_closed() {
        let mut trash = Trash::new(10);
        for item in ["a", "b", "c"] {
            trash.push(item);
        }
        assert_eq!(ids(&trash), vec!["c", "b", "a"]);
        assert_eq!(trash.restore_last(), Some("c"));
        assert_eq!(trash.restore_last(), Some("b"));
        assert_eq!(ids(&trash), vec!["a"]);
        assert_eq!(trash.restore_last(), Some("a"));
        assert_eq!(trash.restore_last(), None);
    }

    #[test]
    fn going_over_the_limit_discards_the_oldest() {
        let mut trash = Trash::new(2);
        assert!(trash.push("a").discarded.is_empty());
        assert!(trash.push("b").discarded.is_empty());
        let pushed = trash.push("c");
        assert_eq!(pushed.discarded, vec!["a"]);
        assert!(pushed.id.is_some());
        assert_eq!(ids(&trash), vec!["c", "b"]);
    }

    #[test]
    fn with_a_limit_of_zero_a_closed_item_is_discarded_at_once() {
        let mut trash = Trash::new(0);
        let pushed = trash.push("a");
        assert_eq!(pushed.id, None);
        assert_eq!(pushed.discarded, vec!["a"]);
        assert_eq!(ids(&trash), Vec::<&str>::new());
    }

    #[test]
    fn lowering_the_limit_discards_the_oldest_and_raising_it_keeps_everything() {
        let mut trash = Trash::new(5);
        for item in ["a", "b", "c", "d"] {
            trash.push(item);
        }
        assert_eq!(trash.set_limit(2), vec!["a", "b"]);
        assert_eq!(ids(&trash), vec!["d", "c"]);
        assert_eq!(trash.set_limit(10), Vec::<&str>::new());
        trash.push("e");
        assert_eq!(ids(&trash), vec!["e", "d", "c"]);
        assert_eq!(trash.set_limit(0), vec!["c", "d", "e"]);
        assert_eq!(ids(&trash), Vec::<&str>::new());
    }

    #[test]
    fn an_item_chosen_from_the_list_is_restored_once() {
        let mut trash = Trash::new(10);
        let a = trash.push("a").id.unwrap();
        trash.push("b");
        let c = trash.push("c").id.unwrap();
        assert_eq!(trash.get_mut(a).copied(), Some("a"));
        assert_eq!(trash.restore(a), Some("a"));
        assert_eq!(trash.get_mut(a), None);
        assert_eq!(trash.restore(a), None);
        assert_eq!(ids(&trash), vec!["c", "b"]);
        assert_eq!(trash.list().next().map(|(id, _)| id), Some(c));
        assert_eq!(trash.restore_last(), Some("c"));
    }

    #[test]
    fn emptying_hands_back_everything() {
        let mut trash = Trash::new(10);
        trash.push("a");
        trash.push("b");
        assert_eq!(trash.clear(), vec!["a", "b"]);
        assert_eq!(ids(&trash), Vec::<&str>::new());
    }
}
