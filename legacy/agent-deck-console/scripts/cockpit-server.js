const express = require('express');
const cors = require('cors');
const fs = require('fs');
const path = require('path');
const { exec } = require('child_process');

const app = express();
const port = 8421;

app.use(cors());
app.use(express.json());
app.use(express.static(path.join(__dirname, '../web')));

const ROOT_DIR = process.env.NODE_ENV === 'production' 
  ? path.join(__dirname, '..') 
  : path.join(__dirname, '..'); // They are actually the same in this structure, but we can add more logic if needed.

// Ensure config files exist if missing
const configDir = path.join(ROOT_DIR, 'config');
if (!fs.existsSync(configDir)) {
  fs.mkdirSync(configDir, { recursive: true });
}

function parseTSV(filePath) {
  const content = fs.readFileSync(filePath, 'utf-8');
  const lines = content.split('\n');
  return lines
    .filter(line => line && !line.startsWith('#'))
    .map(line => line.split('\t'));
}

app.get('/api/config/clis', (req, res) => {
  try {
    const clis = parseTSV(path.join(ROOT_DIR, 'config/ai-clis.tsv'));
    res.json(clis);
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

app.get('/api/config/skills', (req, res) => {
  try {
    const skills = parseTSV(path.join(ROOT_DIR, 'config/ai-skills.tsv'));
    res.json(skills);
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

app.get('/api/usage', (req, res) => {
  try {
    const usage = fs.readFileSync(path.join(ROOT_DIR, 'config/usage-tracker.json'), 'utf-8');
    res.json(JSON.parse(usage));
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

app.get('/api/config/settings', (req, res) => {
  try {
    const settingsPath = path.join(ROOT_DIR, 'config/spartan-settings.env');
    const examplePath = path.join(ROOT_DIR, 'config/spartan-settings.env.example');
    const targetPath = fs.existsSync(settingsPath) ? settingsPath : examplePath;
    
    const content = fs.readFileSync(targetPath, 'utf-8');
    const settings = {};
    content.split('\n').forEach(line => {
      if (line && !line.startsWith('#') && line.includes('=')) {
        const [key, ...value] = line.split('=');
        settings[key.trim()] = value.join('=').trim();
      }
    });
    res.json(settings);
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

app.get('/api/config/neural-link', (req, res) => {
  try {
    const filePath = path.join(ROOT_DIR, 'config/neural-link.json');
    const content = fs.readFileSync(filePath, 'utf-8');
    res.json(JSON.parse(content));
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

app.post('/api/config/neural-link', (req, res) => {
  try {
    const newConfig = req.body;
    const filePath = path.join(ROOT_DIR, 'config/neural-link.json');
    const currentConfig = JSON.parse(fs.readFileSync(filePath, 'utf-8'));
    const mergedConfig = { ...currentConfig, ...newConfig };
    fs.writeFileSync(filePath, JSON.stringify(mergedConfig, null, 2));
    res.json({ success: true });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

app.post('/api/config/settings', (req, res) => {
  try {
    const newSettings = req.body;
    const settingsPath = path.join(ROOT_DIR, 'config/spartan-settings.env');
    const examplePath = path.join(ROOT_DIR, 'config/spartan-settings.env.example');
    
    let currentSettings = {};
    if (fs.existsSync(settingsPath)) {
      const content = fs.readFileSync(settingsPath, 'utf-8');
      content.split('\n').forEach(line => {
        if (line && !line.startsWith('#') && line.includes('=')) {
          const [key, ...value] = line.split('=');
          currentSettings[key.trim()] = value.join('=').trim();
        }
      });
    } else if (fs.existsSync(examplePath)) {
      const content = fs.readFileSync(examplePath, 'utf-8');
      content.split('\n').forEach(line => {
        if (line && !line.startsWith('#') && line.includes('=')) {
          const [key, ...value] = line.split('=');
          currentSettings[key.trim()] = value.join('=').trim();
        }
      });
    }

    const mergedSettings = { ...currentSettings, ...newSettings };
    let content = '# Spartan Settings\n';
    for (const [key, value] of Object.entries(mergedSettings)) {
      content += `${key}=${value}\n`;
    }
    
    fs.writeFileSync(settingsPath, content);
    res.json({ success: true });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

app.get('/api/status', (req, res) => {
  exec('bash ./bin/spartan-agent-deck doctor', { cwd: ROOT_DIR }, (err, stdout, stderr) => {
    res.json({
      output: stdout,
      error: stderr || (err ? err.message : null)
    });
  });
});

app.get('/api/analyze', (req, res) => {
  const target = req.query.path || ROOT_DIR;
  // Basic analysis: count files by extension
  const stats = {};
  function walk(dir) {
    const files = fs.readdirSync(dir);
    for (const file of files) {
      const fullPath = path.join(dir, file);
      if (fs.statSync(fullPath).isDirectory()) {
        if (!['.git', 'node_modules', '.venv'].includes(file)) {
          walk(fullPath);
        }
      } else {
        const ext = path.extname(file);
        stats[ext] = (stats[ext] || 0) + 1;
      }
    }
  }

  try {
    walk(target);
    res.json({ path: target, stats });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

app.listen(port, () => {
  console.log(`Spartan Cockpit Server running at http://localhost:${port}`);
});
