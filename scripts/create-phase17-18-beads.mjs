#!/usr/bin/env node

import { readFileSync, writeFileSync } from 'fs';
import { execSync } from 'child_process';

// Read the ChatGPT response
const response = JSON.parse(readFileSync('.flywheel/browser-response.json', 'utf8'));
const { phase_17, phase_18 } = response.extracted_json;

const allBeads = [...phase_17, ...phase_18];

console.log(`Creating ${allBeads.length} beads (${phase_17.length} Phase 17 + ${phase_18.length} Phase 18)...\n`);

let created = 0;
let errors = [];

for (const bead of allBeads) {
  try {
    console.log(`Creating ${bead.id}: ${bead.title}`);

    // Build the description with all fields
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

    // Escape description for shell
    const escapedDesc = fullDescription.replace(/"/g, '\\"').replace(/\$/g, '\\$');
    const escapedTitle = bead.title.replace(/"/g, '\\"').replace(/\$/g, '\\$');

    // Create the bead
    const cmd = `br create --id "${bead.id}" --title "${escapedTitle}" --description "${escapedDesc}" --type task --priority P1`;

    execSync(cmd, { stdio: 'inherit' });

    created++;
    console.log(`✓ Created ${bead.id}\n`);

  } catch (error) {
    errors.push({ bead: bead.id, error: error.message });
    console.error(`✗ Failed to create ${bead.id}: ${error.message}\n`);
  }
}

// Now set dependencies
console.log('\n--- Setting up dependencies ---\n');

for (const bead of allBeads) {
  if (bead.depends_on && bead.depends_on.length > 0) {
    try {
      console.log(`Setting dependencies for ${bead.id}:`);
      for (const dep of bead.depends_on) {
        const cmd = `br add-dependency --from "${bead.id}" --to "${dep}"`;
        execSync(cmd, { stdio: 'inherit' });
        console.log(`  ✓ ${bead.id} depends on ${dep}`);
      }
    } catch (error) {
      errors.push({ bead: bead.id, error: `Dependency setup failed: ${error.message}` });
      console.error(`  ✗ Failed to set dependencies for ${bead.id}: ${error.message}`);
    }
  }
}

// Summary
console.log('\n=== Summary ===');
console.log(`Created: ${created}/${allBeads.length} beads`);
if (errors.length > 0) {
  console.log(`\nErrors: ${errors.length}`);
  errors.forEach(e => console.log(`  - ${e.bead}: ${e.error}`));
  process.exit(1);
} else {
  console.log('\n✅ All beads created successfully!');

  // Write summary
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
    created_at: new Date().toISOString()
  };

  writeFileSync('tmp/phase17-18-beads-created.json', JSON.stringify(summary, null, 2));
  console.log('\nSummary written to: tmp/phase17-18-beads-created.json');
}
