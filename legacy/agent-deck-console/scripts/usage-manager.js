const fs = require('fs');
const path = require('path');

const USAGE_FILE = path.join(__dirname, '../config/usage-tracker.json');

function getUsage() {
  return JSON.parse(fs.readFileSync(USAGE_FILE, 'utf-8'));
}

function saveUsage(data) {
  fs.writeFileSync(USAGE_FILE, JSON.stringify(data, null, 2));
}

function trackUsage(engine, tokens = 1) {
  const data = getUsage();
  if (data.engines[engine]) {
    data.engines[engine].usage += tokens;
    saveUsage(data);
  }
}

function getBestEngine(requestedEngine) {
  const data = getUsage();
  const engine = data.engines[requestedEngine];

  if (!engine) return requestedEngine;

  const limit = engine.is_paid ? engine.paid_limit : engine.free_limit;
  if (limit === null || engine.usage < limit) {
    return requestedEngine;
  }

  if (data.settings.auto_switch) {
    for (const fallback of data.settings.fallback_order) {
      const fEngine = data.engines[fallback];
      const fLimit = fEngine.is_paid ? fEngine.paid_limit : fEngine.free_limit;
      if (fLimit === null || fEngine.usage < fLimit) {
        console.error(`Spartan: ${requestedEngine} limit reached. Switching to ${fallback}.`);
        return fallback;
      }
    }
  }

  return requestedEngine;
}

const args = process.argv.slice(2);
const command = args[0];

if (command === 'track') {
  trackUsage(args[1], parseInt(args[2] || 1));
} else if (command === 'resolve') {
  console.log(getBestEngine(args[1]));
}
