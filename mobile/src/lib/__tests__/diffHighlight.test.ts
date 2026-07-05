import { parseDiff } from '../diffHighlight';

describe('parseDiff', () => {
  test('an empty patch parses to no lines', () => {
    expect(parseDiff('')).toEqual([]);
  });

  test('a hunk header line is typed as hunk', () => {
    const lines = parseDiff('@@ -1,2 +1,2 @@ function foo() {\n');
    expect(lines).toEqual([{ type: 'hunk', text: '@@ -1,2 +1,2 @@ function foo() {' }]);
  });

  test('a line starting with + is typed as add', () => {
    const lines = parseDiff('+  const x = 1;\n');
    expect(lines).toEqual([{ type: 'add', text: '+  const x = 1;' }]);
  });

  test('a line starting with - is typed as remove', () => {
    const lines = parseDiff('-  const x = 1;\n');
    expect(lines).toEqual([{ type: 'remove', text: '-  const x = 1;' }]);
  });

  test('a line with no leading marker is typed as context', () => {
    const lines = parseDiff('  const y = 2;\n');
    expect(lines).toEqual([{ type: 'context', text: '  const y = 2;' }]);
  });

  test('a full patch parses each line to its own type in order', () => {
    const patch =
      '@@ -22,7 +22,10 @@ export async function commitOrder(order: PendingOrder) {\n' +
      '-  await db.orders.insert(order);\n' +
      '+  const latestStock = await db.stock.read(order.sku);\n' +
      '+  await db.orders.insert(order);\n';

    expect(parseDiff(patch)).toEqual([
      { type: 'hunk', text: '@@ -22,7 +22,10 @@ export async function commitOrder(order: PendingOrder) {' },
      { type: 'remove', text: '-  await db.orders.insert(order);' },
      { type: 'add', text: '+  const latestStock = await db.stock.read(order.sku);' },
      { type: 'add', text: '+  await db.orders.insert(order);' },
    ]);
  });
});
