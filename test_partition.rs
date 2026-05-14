fn main() {
    let items = vec![
        (true, 1),
        (false, 2),
        (true, 3),
        (false, 4),
        (true, 5),
    ];
    let (trues, falses): (Vec<_>, Vec<_>) = items.into_iter().partition(|(b, _)| *b);
    println!("trues: {:?}", trues);
    println!("falses: {:?}", falses);
}
