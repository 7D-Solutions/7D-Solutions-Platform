#!/usr/bin/env node

import { readFileSync, appendFileSync, writeFileSync } from 'fs';
import { execSync } from 'child_process';

console.log('=== Finalizing Phase 17 & 18 Beads ===\n');

// Read scaffolding beads
const scaffoldingData = JSON.parse(readFileSync('tmp/chatgpt-scaffolding-response.json', 'utf8'));
const { scaffolding_beads, dependency_updates } = scaffoldingData.extracted_json;

const now = new Date().toISOString();

// Step 1: Create scaffolding beads
console.log('Step 1: Creating 7 scaffolding beads...\n');

for (const bead of scaffolding_beads) {
  const fullDescription = `${bead.description}

## How to Think
${bead.how_to_think}

## Acceptance Criteria
${bead.acceptance_criteria.map((c, i) => `${i + 1}. ${c}`).join('\n')}

## Verification Commands
${bead.verification_commands.map((c, i) => `${i + 1}. ${c}`).join('\n')}

## Files Involved
${bead.files_involved.map(f => `- ${f}`).join('\n')}

## Estimated Complexity
${bead.estimated_complexity}/5

## Dependencies
None (foundation scaffolding bead)
`;

  const entry = {
    id: bead.id,
    title: bead.title,
    description: fullDescription,
    status: "open",
    priority: 0, // P0 for scaffolding
    issue_type: "task",
    created_at: now,
    created_by: "BrightHill",
    updated_at: now,
    source_repo: ".",
    compaction_level: 0,
    original_size: 0
  };

  appendFileSync('.beads/issues.jsonl', JSON.stringify(entry) + '\n');
  console.log(`✓ Created ${bead.id}: ${bead.title}`);
}

console.log('\n✓ All scaffolding beads written to JSONL\n');

// Import scaffolding beads
console.log('Importing scaffolding beads into database...');
execSync('br sync --import-only', { stdio: 'inherit' });

// Step 2: Add dependencies to database
console.log('\n\nStep 2: Adding scaffold dependencies to existing beads...\n');

const db_cmd = (issue_id, dep_id) => {
  const cmd = `sqlite3 .beads/beads.db "INSERT OR IGNORE INTO dependencies (issue_id, depends_on_id, type, created_at, created_by) VALUES ('${issue_id}', '${dep_id}', 'blocks', '${now}', 'BrightHill');"`;
  execSync(cmd);
};

for (const update of dependency_updates) {
  console.log(`${update.bead_id} now depends on:`);
  for (const dep of update.add_dependencies) {
    db_cmd(update.bead_id, dep);
    console.log(`  ✓ ${dep}`);
  }
}

console.log('\n✓ All scaffold dependencies added\n');

// Step 3: Update priorities
console.log('Step 3: Updating priorities...\n');

// Phase 17: All P0
const phase17_beads = [
  'bd-17a0', 'bd-17a1', 'bd-17a2', 'bd-17a3', 'bd-17a4',
  'bd-17b0', 'bd-17b1', 'bd-17b2', 'bd-17b3', 'bd-17b4',
  'bd-17c0', 'bd-17c1', 'bd-17c2', 'bd-17c3', 'bd-17c4',
  'bd-17s0', 'bd-17s1', 'bd-17s2', 'bd-17s3', 'bd-17s4'
];

// Phase 18 P0: security + DR + overload
const phase18_p0 = ['bd-18b0', 'bd-18b1', 'bd-18c1', 'bd-18d0', 'bd-18s0'];

// Phase 18 P1: release/dashboards/export
const phase18_p1 = ['bd-18a0', 'bd-18a1', 'bd-18c0', 'bd-18e0', 'bd-18s1'];

for (const bead_id of phase17_beads) {
  execSync(`sqlite3 .beads/beads.db "UPDATE issues SET priority = 0 WHERE id = '${bead_id}';"`);
  console.log(`✓ ${bead_id} → P0`);
}

for (const bead_id of phase18_p0) {
  execSync(`sqlite3 .beads/beads.db "UPDATE issues SET priority = 0 WHERE id = '${bead_id}';"`);
  console.log(`✓ ${bead_id} → P0`);
}

for (const bead_id of phase18_p1) {
  execSync(`sqlite3 .beads/beads.db "UPDATE issues SET priority = 1 WHERE id = '${bead_id}';"`);
  console.log(`✓ ${bead_id} → P1`);
}

console.log('\n✓ All priorities updated\n');

// Step 4: Flush to JSONL
console.log('Step 4: Flushing to JSONL...');
execSync('br sync --flush-only', { stdio: 'inherit' });

console.log('\n\n=== Summary ===');
console.log(`Scaffolding beads created: ${scaffolding_beads.length}`);
console.log(`Dependency updates applied: ${dependency_updates.length}`);
console.log(`Priority updates: ${phase17_beads.length + phase18_p0.length + phase18_p1.length}`);
console.log('\n✅ Phase 17 & 18 beads finalized!');
console.log('\n📊 Total beads: 30 (7 scaffolding + 23 implementation)');
console.log('   - Phase 17: 20 beads (5 scaffolding + 15 implementation)');
console.log('   - Phase 18: 10 beads (2 scaffolding + 8 implementation)');
