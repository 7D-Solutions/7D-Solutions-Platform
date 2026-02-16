#!/usr/bin/env node

import { readFileSync } from 'fs';
import { execSync } from 'child_process';
import Database from 'better-sqlite3';

// Read the ChatGPT response
const response = JSON.parse(readFileSync('.flywheel/browser-response.json', 'utf8'));
const { phase_17, phase_18 } = response.extracted_json;

const allBeads = [...phase_17, ...phase_18];

console.log('Setting up dependencies for Phase 17 & 18 beads...\n');

// Open database
const db = new Database('.beads/beads.db');

const now = new Date().toISOString();
let added = 0;

for (const bead of allBeads) {
  if (bead.depends_on && bead.depends_on.length > 0) {
    console.log(`${bead.id} depends on:`);

    for (const dep of bead.depends_on) {
      try {
        // Insert dependency
        // Note: issue_id depends on depends_on_id
        // So if bd-17a1 depends on bd-17a0, then:
        // - depends_on_id = bd-17a0 (the parent)
        // - issue_id = bd-17a1 (the child that depends on parent)

        db.prepare(`
          INSERT OR IGNORE INTO dependencies (issue_id, depends_on_id, type, created_at, created_by)
          VALUES (?, ?, 'blocks', ?, 'BrightHill')
        `).run(bead.id, dep, now);

        console.log(`  ✓ ${dep} → ${bead.id}`);
        added++;
      } catch (error) {
        console.error(`  ✗ Failed to add ${dep}: ${error.message}`);
      }
    }
  }
}

db.close();

console.log(`\n✅ Added ${added} dependencies`);

// Flush to JSONL
console.log('\nFlushing to JSONL...');
execSync('br sync --flush-only', { stdio: 'inherit' });

console.log('\n✅ Dependencies setup complete!');
