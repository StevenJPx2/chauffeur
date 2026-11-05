import { expect, test } from "bun:test"
import { carrier } from "../src/carry.ts"

test("text is taken once, for its call", () => {
  const carried = carrier()

  carried.put("a", ["one"])
  carried.put("a", ["two"])
  carried.put("b", [])

  expect(carried.take("b")).toEqual([])
  expect(carried.take("a")).toEqual(["one", "two"])
  expect(carried.take("a")).toEqual([])
})

test("only the latest calls are kept", () => {
  const carried = carrier(2)

  carried.put("a", ["one"])
  carried.put("b", ["two"])
  carried.put("c", ["three"])

  expect(carried.take("a")).toEqual([])
  expect(carried.take("b")).toEqual(["two"])
  expect(carried.take("c")).toEqual(["three"])
})
