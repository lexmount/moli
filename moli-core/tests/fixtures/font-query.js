async function() {
  const rows = [], failures = [];
  const cases = [
  [
    "",
    "SyntaxError",
    "SyntaxError"
  ],
  [
    "inherit",
    "SyntaxError",
    "SyntaxError"
  ],
  [
    "default",
    "SyntaxError",
    "SyntaxError"
  ],
  [
    "12px inherit",
    "SyntaxError",
    "SyntaxError"
  ],
  [
    "12px default",
    "SyntaxError",
    "SyntaxError"
  ],
  [
    "\"inherit\"",
    "SyntaxError",
    "SyntaxError"
  ],
  [
    "\"default\"",
    "SyntaxError",
    "SyntaxError"
  ],
  [
    "12px",
    "SyntaxError",
    "SyntaxError"
  ],
  [
    "serif",
    "SyntaxError",
    "SyntaxError"
  ],
  [
    "12px serif; color: red",
    "SyntaxError",
    "SyntaxError"
  ],
  [
    "-1px serif",
    "SyntaxError",
    "SyntaxError"
  ],
  [
    "var(--x) serif",
    "SyntaxError",
    "SyntaxError"
  ],
  [
    "var(--x, 10px) serif",
    "SyntaxError",
    "SyntaxError"
  ],
  [
    "env(size) serif",
    "SyntaxError",
    "SyntaxError"
  ],
  [
    "12px serif !important",
    "SyntaxError",
    "SyntaxError"
  ],
  [
    "12px serif junk,",
    "SyntaxError",
    "SyntaxError"
  ],
  [
    "normal normal normal normal 12px serif",
    "loaded",
    true
  ],
  [
    "12px \"inherit\"",
    "loaded",
    true
  ],
  [
    "12px \"default\"",
    "loaded",
    true
  ],
  [
    "12px \"revert\"",
    "loaded",
    true
  ],
  [
    "italic 700 16px/1.2 \"A B\", serif",
    "loaded",
    true
  ],
  [
    "calc(1em + 2px) serif",
    "loaded",
    true
  ],
  [
    "caption",
    "loaded",
    true
  ],
  [
    "12px serif",
    "loaded",
    true
  ]
];
  for (const [query, expectedLoad, expectedCheck] of cases) {
    const load = await document.fonts.load(query).then(() => 'loaded', error => error.name);
    let check;
    try { check = document.fonts.check(query); } catch (error) { check = error.name; }
    const row = [query, load, check];
    rows.push(row);
    if (load !== expectedLoad || check !== expectedCheck) failures.push({query, load, check, expectedLoad, expectedCheck});
  }
  return {rows, failures};
}
