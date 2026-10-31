(function geometryMatrixConstructorUnion(realm, worker) {
  const checks = [];
  const check = (name, body) => {
    try {
      if (body() !== true) throw Error("unexpected result");
      checks.push({ name, passed: true });
    } catch (error) {
      checks.push({ name, passed: false, error: String(error) });
    }
  };
  const six = [2, 3, 4, 5, 10, 20];
  const values = (matrix) => [
    matrix.a,
    matrix.b,
    matrix.c,
    matrix.d,
    matrix.e,
    matrix.f,
  ];
  const equal = (actual, expected) =>
    actual.length === expected.length &&
    actual.every((value, index) => Object.is(value, expected[index]));
  const throws = (body, name) => {
    try {
      body();
    } catch (error) {
      return (
        error.name === name &&
        (name !== "TypeError" ||
          Object.getPrototypeOf(error) === realm.TypeError.prototype)
      );
    }
    return false;
  };
  const propagates = (body, sentinel) => {
    try {
      body();
    } catch (error) {
      return error === sentinel;
    }
    return false;
  };
  for (const name of ["DOMMatrix", "DOMMatrixReadOnly"]) {
    const Ctor = realm[name];
    const test = (suffix, body) => check(name + "/" + suffix, body);
    test("omitted", () => new Ctor().isIdentity);
    test("undefined", () => new Ctor(undefined).isIdentity);
    test("plain-object-is-string", () =>
      throws(
        () => new Ctor({ is2D: false }),
        worker ? "TypeError" : "SyntaxError",
      ));
    test("null-is-string", () =>
      throws(() => new Ctor(null), worker ? "TypeError" : "SyntaxError"));
    test("symbol", () => throws(() => new Ctor(Symbol()), "TypeError"));
    test("string", () =>
      worker
        ? throws(() => new Ctor("matrix(2,3,4,5,10,20)"), "TypeError")
        : equal(values(new Ctor("matrix(2,3,4,5,10,20)")), six));
    test("iterable", () =>
      equal(
        values(
          new Ctor({
            *[Symbol.iterator]() {
              yield* six;
            },
            toString() {
              throw Error("string branch");
            },
          }),
        ),
        six,
      ));
    test("function-iterable", () => {
      const init = function () {};
      init[Symbol.iterator] = function* () {
        yield* six;
      };
      return equal(values(new Ctor(init)), six);
    });
    test("boxed-string-iterable", () =>
      equal(values(new Ctor(new realm.String("123456"))), [1, 2, 3, 4, 5, 6]));
    test("boxed-string-overridden-iterator", () => {
      const init = new realm.String("not a matrix");
      init[Symbol.iterator] = function* () {
        yield* six;
      };
      return equal(values(new Ctor(init)), six);
    });
    test("boxed-string-no-iterator", () => {
      const init = new realm.String("matrix(2,3,4,5,10,20)");
      init[Symbol.iterator] = null;
      return worker
        ? throws(() => new Ctor(init), "TypeError")
        : equal(values(new Ctor(init)), six);
    });
    test("set", () =>
      equal(values(new Ctor(new Set([1, 2, 3, 4, 5, 6]))), [1, 2, 3, 4, 5, 6]));
    test("uint8-sequence", () =>
      equal(values(new Ctor(new realm.Uint8Array(six))), six));
    test("float-array-uses-iterator", () => {
      const init = new realm.Float64Array(6);
      init[Symbol.iterator] = function* () {
        yield* six;
      };
      return equal(values(new Ctor(init)), six);
    });
    test("array-uses-indexed-elements", () => {
      const init = [0, 0, 0, 0, 0, 0];
      init[Symbol.iterator] = function* () {
        yield* six;
      };
      return equal(values(new Ctor(init)), [0, 0, 0, 0, 0, 0]);
    });
    test("array-iterator-getter-is-skipped", () => {
      const init = six.slice();
      Object.defineProperty(init, Symbol.iterator, {
        get() {
          throw Error("array fast path");
        },
      });
      return equal(values(new Ctor(init)), six);
    });
    test("array-length-validation-after-conversion", () => {
      let converted = 0;
      const init = Array.from({ length: 7 }, (_, i) => ({
        valueOf() {
          converted++;
          return i;
        },
      }));
      return throws(() => new Ctor(init), "TypeError") && converted === 7;
    });
    test("array-element-exception", () => {
      const sentinel = {};
      const init = [
        {
          valueOf() {
            throw sentinel;
          },
        },
        0,
        0,
        0,
        0,
        0,
      ];
      return propagates(() => new Ctor(init), sentinel);
    });
    test("iterator-getter-once", () => {
      let count = 0;
      const init = {
        get [Symbol.iterator]() {
          if (++count !== 1) throw Error("read twice");
          return function* () {
            yield* six;
          };
        },
      };
      return equal(values(new Ctor(init)), six) && count === 1;
    });
    test("string-conversion-order", () => {
      const order = [];
      const init = {
        get [Symbol.iterator]() {
          order.push("iterator");
          return undefined;
        },
        [Symbol.toPrimitive](hint) {
          order.push(hint);
          return "matrix(2,3,4,5,10,20)";
        },
      };
      const result = worker
        ? throws(() => new Ctor(init), "TypeError")
        : equal(values(new Ctor(init)), six);
      return result && order.join(",") === "iterator,string";
    });
    test("noncallable-iterator", () => {
      let converted = false;
      return (
        throws(
          () =>
            new Ctor({
              [Symbol.iterator]: 1,
              toString() {
                converted = true;
                return "";
              },
            }),
          "TypeError",
        ) && !converted
      );
    });
    test("iterator-getter-exception", () => {
      const sentinel = {};
      return propagates(
        () =>
          new Ctor({
            get [Symbol.iterator]() {
              throw sentinel;
            },
          }),
        sentinel,
      );
    });
    test("iterator-method-exception", () => {
      const sentinel = {};
      return propagates(
        () =>
          new Ctor({
            [Symbol.iterator]() {
              throw sentinel;
            },
          }),
        sentinel,
      );
    });
    test("string-conversion-exception", () => {
      const sentinel = {};
      return propagates(
        () =>
          new Ctor({
            toString() {
              throw sentinel;
            },
          }),
        sentinel,
      );
    });
    test("interleaved-iteration-and-conversion", () => {
      const order = [];
      let index = 0;
      const init = {
        [Symbol.iterator]() {
          order.push("iterator");
          return {
            next() {
              const i = index++;
              order.push("next" + i);
              return i < 6
                ? {
                    value: {
                      valueOf() {
                        order.push("value" + i);
                        return six[i];
                      },
                    },
                    done: false,
                  }
                : { done: true };
            },
          };
        },
      };
      const matrix = new Ctor(init);
      const expected = ["iterator"];
      for (let i = 0; i < 6; i++) expected.push("next" + i, "value" + i);
      expected.push("next6");
      return (
        equal(values(matrix), six) && order.join(",") === expected.join(",")
      );
    });
    test("length-validation-after-conversion", () => {
      let converted = 0;
      const init = {
        [Symbol.iterator]: function* () {
          for (let i = 0; i < 7; i++)
            yield {
              valueOf() {
                converted++;
                return i;
              },
            };
        },
      };
      return throws(() => new Ctor(init), "TypeError") && converted === 7;
    });
    test("element-error-stops-without-close", () => {
      let next = 0,
        closed = 0;
      const sentinel = {};
      const init = {
        [Symbol.iterator]() {
          return {
            next() {
              next++;
              return {
                done: false,
                value: {
                  valueOf() {
                    throw sentinel;
                  },
                },
              };
            },
            return() {
              closed++;
              return { done: true };
            },
          };
        },
      };
      return (
        propagates(() => new Ctor(init), sentinel) && next === 1 && closed === 0
      );
    });
    test("empty-sequence", () => throws(() => new Ctor([]), "TypeError"));
    test("six-and-sixteen-dimensions", () => {
      const a = new Ctor(six),
        b = new Ctor([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]);
      return a.is2D === true && b.is2D === false;
    });
    test("unrestricted-double-sequence", () => {
      const m = new Ctor([Infinity, NaN, -0, 2, -Infinity, 0]);
      return (
        m.a === Infinity &&
        Number.isNaN(m.b) &&
        Object.is(m.c, -0) &&
        m.e === -Infinity
      );
    });
    test("fromMatrix-still-uses-dictionary", () => {
      const init = {
        a: 2,
        d: 3,
        e: 4,
        get [Symbol.iterator]() {
          throw Error("not a constructor union");
        },
      };
      return equal(values(Ctor.fromMatrix(init)), [2, 0, 0, 3, 4, 0]);
    });
    test("no-new-before-conversion", () => {
      let touched = false;
      return (
        throws(
          () =>
            Ctor({
              get [Symbol.iterator]() {
                touched = true;
                return undefined;
              },
            }),
          "TypeError",
        ) && !touched
      );
    });
  }
  return {
    total: checks.length,
    failures: checks.filter((row) => !row.passed),
    checks,
  };
});
