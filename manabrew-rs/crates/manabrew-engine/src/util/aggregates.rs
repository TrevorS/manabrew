use crate::game_rng::GameRng;

pub fn random<T: Copy>(source: &[T], count: usize, rng: &mut dyn GameRng) -> Vec<T> {
    let mut list = Vec::with_capacity(count.min(source.len()));
    for (index, &item) in source.iter().enumerate() {
        let i = index + 1;
        if i <= count {
            list.push(item);
        } else {
            let j = rng.next_int(i as i32) as usize;
            if j < count {
                list[j] = item;
            }
        }
    }
    list
}
