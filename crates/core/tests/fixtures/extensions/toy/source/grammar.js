// Rebuild grammars/toy.wasm from here: tree-sitter generate && tree-sitter build --wasm -o ../grammars/toy.wasm .
module.exports = grammar({
  name: 'toy',
  word: $ => $.identifier,
  extras: $ => [/\s/, $.comment],
  rules: {
    source_file: $ => repeat(choice($.statement, $.identifier, $.string, $.number)),
    statement: $ => seq(choice('let', 'fn'), $.identifier),
    identifier: _ => /[a-z_]+/,
    string: _ => /"[^"\n]*"/,
    number: _ => /\d+/,
    comment: _ => token(seq('#', /.*/)),
  },
});
