use rand::Rng;
use std::{cmp::Ordering, fs::File, io::BufReader};
use word2vec::vectorreader::WordVectorReader;

pub struct Words {
    pub vocabulary: Vec<(String, Vec<f32>)>,
}

impl Words {
    pub fn load(word2vec_model_file: &str) -> anyhow::Result<Words> {
        let _span = trace_span!("Loading word2vec db").entered();

        let file = File::open(word2vec_model_file)?;
        let reader = WordVectorReader::new_from_reader(BufReader::new(file))?;

        let mut vocabulary = Vec::with_capacity(reader.vocabulary_size());

        for (word, vec) in reader {
            vocabulary.push((word.clone(), vec));
        }

        Ok(Words { vocabulary })
    }

    pub fn vector(&self, word: &str) -> Option<&[f32]> {
        self.vocabulary
            .iter()
            .position(|x| x.0.as_str() == word)
            .map(|index| &self.vocabulary[index].1[..])
    }

    /// `!thesaurus <word> <count>`
    pub fn thesaurus(&self, word: &str, count: usize) -> String {
        let _span = trace_span!("thesaurus", word, count).entered();

        let results = match self.vector(word) {
            Some(val) => {
                let mut metrics: Vec<(usize, f32)> = Vec::with_capacity(self.vocabulary.len());
                metrics.extend(
                    self.vocabulary
                        .iter()
                        .enumerate()
                        .map(|(i, other_val)| (i, val.iter().zip(other_val.1.iter()).map(|(&a, &b)| a * b).sum())),
                );

                metrics.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));
                Some(
                    metrics[1..count + 1]
                        .iter()
                        .map(|&(idx, dist)| (self.vocabulary[idx].clone().0, dist))
                        .collect::<Vec<_>>(),
                )
            }
            None => None,
        };

        format!("{:?}", results)
    }

    /// Picks a random word from the vocabulary.
    pub fn pick_word(&self) -> String {
        let mut rng = rand::thread_rng();
        let pos: usize = rng.gen_range(0..self.vocabulary.len());
        self.vocabulary[pos].0.clone()
    }
}
