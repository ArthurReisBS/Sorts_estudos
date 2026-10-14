fn bubbleSort(v: &mut Vec<i32>, n: usize){
    let mut fim = n;
    let mut continua = 1;
    let mut aux;

    while (continua != 0) {
        continua = 0;
        for i in 0..fim - 1 {
            if (v[i] > v[i+1]) {
                aux = v[i];
                v[i] = v[i+1];
                v[i+1] = aux;
                continua = i;
            }
        }
        fim -= 1;
    }
}