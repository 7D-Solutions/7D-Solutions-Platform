#!/usr/bin/env node

import { readFileSync, writeFileSync, appendFileSync } from 'fs';
import { execSync } from 'child_process';

// Read the ChatGPT response
const response = JSON.parse(readFileSync('.flywheel/browser-response.json', 'utf8'));
const { phase_17, phase_18 } = response.extracted_json;

const allBeads = [...phase_17, ...phase_18];

console.log(`Creating ${allBeads.length} beads (${phase_17.length} Phase 17 + ${phase_18.length} Phase 18)...\n`);

const now = new Date().toISOString();
const jsonlEntries = [];

for (const bead of allBeads) {
  // Build the full description
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
${bead.depends_on.length > 0 ? bead.depends_on.map(d => `- ${d}`).join('\n') : 'None (foundation bead)'}
`;

  const entry = {
    id: bead.id,
    title: bead.title,
    description: fullDescription,
    status: "open",
    priority: 1, // P1 for all Phase 17/18 beads
    issue_type: "task",
    created_at: now,
    created_by: "BrightHill",
    updated_at: now,
    source_repo: ".",
    compaction_level: 0,
    original_size: 0
  };

  jsonlEntries.push(entry);
  console.log(`Prepared ${bead.id}: ${bead.title}`);
}

// Append to JSONL file
console.log(`\nAppending ${jsonlEntries.length} entries to .beads/issues.jsonl...`);
for (const entry of jsonlEntries) {
  appendFileSync('.beads/issues.jsonl', JSON.stringify(entry) + '\n');
}

console.log('✓ Entries written to JSONL\n');

// Import into database
console.log('Importing into database...');
try {
  execSync('br sync --import-only', { stdio: 'inherit' });
  console.log('\n✓ Database import complete!');
} catch (error) {
  console.error('\n✗ Database import failed:', error.message);
  process.exit(1);
}

// Now set up dependencies using SQL directly or via br commands
console.log('\n--- Setting up dependencies ---\n');

// Dependencies need to be added to the database
// Let's check if there's a way to do this via SQL or if we need to use br commands

// For now, let's document the dependencies
const depMap = {};
for (const bead of allBeads) {
  if (bead.depends_on && bead.depends_on.length > 0) {
    depMap[bead.id] = bead.depends_on;
  }
}

writeFileSync('tmp/phase17-18-dependencies.json', JSON.stringify(depMap, null, 2));
console.log('✓ Dependencies documented in tmp/phase17-18-dependencies.json');

// Summary
const summary = {
  phase_17: {
    total: phase_17.length,
    tracks: {
      projection_integrity: phase_17.filter(b => b.id.startsWith('bd-17a')).length,
      audit_spine: phase_17.filter(b => b.id.startsWith('bd-17b')).length,
      tenant_provisioning: phase_17.filter(b => b.id.startsWith('bd-17c')).length
    }
  },
  phase_18: {
    total: phase_18.length
  },
  beads_created: allBeads.map(b => ({ id: b.id, title: b.title, deps: b.depends_on })),
  created_at: now
};

writeFileSync('tmp/phase17-18-beads-created.json', JSON.stringify(summary, null, 2));
console.log('✓ Summary written to tmp/phase17-18-beads-created.json');

console.log('\n=== Summary ===');
console.log(`Phase 17: ${phase_17.length} beads`);
console.log(`  - Track A (Projection): ${phase_17.filter(b => b.id.startsWith('bd-17a')).length} beads`);
console.log(`  - Track B (Audit): ${phase_17.filter(b => b.id.startsWith('bd-17b')).length} beads`);
console.log(`  - Track C (Tenant): ${phase_17.filter(b => b.id.startsWith('bd-17c')).length} beads`);
console.log(`Phase 18: ${phase_18.length} beads`);
console.log(`\n✅ All beads imported successfully!`);
console.log(`\nNote: Dependencies documented but need to be set up separately.`);
console.log(`Run the following to set up dependencies manually or via SQL.`);
