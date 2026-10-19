import { add } from './math';
import * as m from './math';
import makeCircle from './shapes/circle';
import { Circle } from './shapes';
import { readFile } from 'fs';

export function main(): void {
  add(1, 2);
  m.mul(2, 3);
  makeCircle();
  const c = new Circle();
  c.area();
  readFile('x');
  helperHere();
}

function helperHere(): void {}
