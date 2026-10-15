fn bubbleSort(v: &mut Vec<i32>, n: usize) {
    let mut menor;
    let mut troca;

    for i in 0..n - 1 {
        menor = i;
        for j in i+1..n {
            if v[j] < v[menor] {
                menor = j;
            }
        }
        if i != menor {
            troca = v[i];
            v[i] = v[menor];
            v[menor] = troca;
        }
    }
}