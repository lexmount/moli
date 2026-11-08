(function geometryPointTransform(realm) {
  const checks = [];
  const check = (name, body) => {
    try {
      if (body() !== true) throw Error("unexpected result");
      checks.push({ name, passed: true });
    } catch (error) {
      checks.push({ name, passed: false, error: String(error) });
    }
  };
  const coordinates = (point) => [point.x, point.y, point.z, point.w];
  const equal = (point, expected) =>
    coordinates(point).every((value, i) => Object.is(value, expected[i]));
  const cases = [
    ["infinite-x", [Infinity, 0], {}, [Infinity, NaN, 0, 1]],
    ["negative-infinite-x", [-Infinity, 0], {}, [-Infinity, NaN, 0, 1]],
    ["nan-x", [NaN, 0], {}, [NaN, NaN, 0, 1]],
    ["infinite-y", [0, Infinity], {}, [NaN, Infinity, 0, 1]],
    ["negative-infinite-y", [0, -Infinity], {}, [NaN, -Infinity, 0, 1]],
    ["nan-y", [0, NaN], {}, [NaN, NaN, 0, 1]],
    ["negative-zero-z-is-2d", [Infinity, 0, -0, 1], {}, [Infinity, NaN, 0, 1]],
    ["explicit-2d", [Infinity, 0], { is2D: true }, [Infinity, NaN, 0, 1]],
    [
      "forced-3d-identity",
      [Infinity, 0],
      { is2D: false },
      [Infinity, NaN, NaN, NaN],
    ],
    ["nonzero-z", [Infinity, 0, 2, 1], {}, [Infinity, NaN, NaN, NaN]],
    ["zero-w", [Infinity, 0, 0, 0], {}, [Infinity, NaN, NaN, NaN]],
    ["negative-zero-w", [Infinity, 0, 0, -0], {}, [Infinity, NaN, NaN, NaN]],
    ["nan-z", [1, 2, NaN, 1], {}, [NaN, NaN, NaN, NaN]],
    ["nan-w", [1, 2, 0, NaN], {}, [NaN, NaN, NaN, NaN]],
    [
      "3d-perspective",
      [Infinity, 0],
      { m14: 1 },
      [Infinity, NaN, NaN, Infinity],
    ],
    [
      "finite-2d",
      [2, 3],
      { a: 2, b: 3, c: 4, d: 5, e: 10, f: 20 },
      [26, 41, 0, 1],
    ],
    ["finite-3d", [2, 3, 4, 2], { m31: 5, m32: 6, m43: 7 }, [22, 27, 18, 2]],
    ["infinite-coefficient", [1, 2], { a: Infinity }, [Infinity, 2, 0, 1]],
    ["nan-coefficient", [1, 2], { a: NaN }, [NaN, 2, 0, 1]],
    [
      "negative-zero-result",
      [1, 1],
      { a: -0, b: -0, c: -0, d: -0, e: -0, f: -0 },
      [-0, -0, 0, 1],
    ],
    ["negative-zero-point", [-0, -0, -0, 1], {}, [0, 0, 0, 1]],
  ];
  for (const [name, args, init, expected] of cases) {
    for (const Point of [realm.DOMPoint, realm.DOMPointReadOnly]) {
      check(Point.name + "/matrixTransform/" + name, () => {
        const result = new Point(...args).matrixTransform(init);
        return (
          Object.getPrototypeOf(result) === realm.DOMPoint.prototype &&
          equal(result, expected)
        );
      });
    }
    for (const Matrix of [realm.DOMMatrix, realm.DOMMatrixReadOnly]) {
      check(Matrix.name + "/transformPoint/" + name, () => {
        const point = new realm.DOMPoint(...args);
        const result = Matrix.fromMatrix(init).transformPoint(point);
        return (
          Object.getPrototypeOf(result) === realm.DOMPoint.prototype &&
          equal(result, expected)
        );
      });
    }
  }
  for (const Matrix of [realm.DOMMatrix, realm.DOMMatrixReadOnly]) {
    for (const [name, components, expected] of [
      ["six-elements-are-2d", [1, 0, 0, 1, 0, 0], [Infinity, NaN, 0, 1]],
      [
        "sixteen-elements-stay-3d",
        [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1],
        [Infinity, NaN, NaN, NaN],
      ],
    ]) {
      check(Matrix.name + "/" + name, () =>
        equal(new Matrix(components).transformPoint({ x: Infinity }), expected),
      );
    }
  }
  return {
    total: checks.length,
    failures: checks.filter((row) => !row.passed),
    checks,
  };
});
