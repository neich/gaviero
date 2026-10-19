export class Circle {
  area(): number {
    return 3;
  }
}

export default function makeCircle(): Circle {
  return new Circle();
}
