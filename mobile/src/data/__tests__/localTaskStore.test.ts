describe('localTaskStore', () => {
  beforeEach(() => {
    jest.resetModules();
  });

  function load() {
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    return require('../localTaskStore');
  }

  test('addLocalTask creates a running thread attributed to on-device dictation', () => {
    const { addLocalTask } = load();
    const thread = addLocalTask('Fix the bug');
    expect(thread.title).toBe('Fix the bug');
    expect(thread.status).toBe('running');
    expect(thread.workspaceName).toBe('Dictated on-device');
    expect(thread.unreadCount).toBe(0);
  });

  test('getLocalTasks reflects every task added, newest first', () => {
    const { addLocalTask, getLocalTasks } = load();
    const first = addLocalTask('first');
    const second = addLocalTask('second');
    expect(getLocalTasks()).toEqual([second, first]);
  });

  test('subscribeLocalTasks notifies listeners on every add', () => {
    const { addLocalTask, subscribeLocalTasks } = load();
    const listener = jest.fn();
    subscribeLocalTasks(listener);
    addLocalTask('one');
    addLocalTask('two');
    expect(listener).toHaveBeenCalledTimes(2);
  });

  test('the unsubscribe function stops further notifications', () => {
    const { addLocalTask, subscribeLocalTasks } = load();
    const listener = jest.fn();
    const unsubscribe = subscribeLocalTasks(listener);
    addLocalTask('one');
    unsubscribe();
    addLocalTask('two');
    expect(listener).toHaveBeenCalledTimes(1);
  });
});
