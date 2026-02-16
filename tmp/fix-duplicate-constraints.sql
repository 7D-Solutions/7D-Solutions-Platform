-- For bd-18a0, extract description up to first SCOPE CONSTRAINT marker
UPDATE issues
SET description = substr(
  description, 
  1, 
  instr(description, '## ⚠️ SCOPE CONSTRAINT') - 1
) || substr(
  description,
  instr(description, '## ⚠️ SCOPE CONSTRAINT'),
  instr(substr(description, instr(description, '## ⚠️ SCOPE CONSTRAINT')), 
       'Keep implementation to basic artifact build + checksum.') + 
  length('Keep implementation to basic artifact build + checksum.') - 1
)
WHERE id = 'bd-18a0' AND instr(description, '## ⚠️ SCOPE CONSTRAINT') > 0;

-- For scaffolding beads, similar approach
UPDATE issues
SET description = substr(
  description, 
  1, 
  instr(description, '## ⚠️ SCAFFOLDING CONSTRAINTS') - 1
) || substr(
  description,
  instr(description, '## ⚠️ SCAFFOLDING CONSTRAINTS'),
  instr(substr(description, instr(description, '## ⚠️ SCAFFOLDING CONSTRAINTS')), 
       'This bead prepares the crate structure.') + 
  length('This bead prepares the crate structure.') - 1
)
WHERE id IN ('bd-17s0', 'bd-17s1', 'bd-17s2', 'bd-17s3', 'bd-17s4', 'bd-18s0', 'bd-18s1')
  AND instr(description, '## ⚠️ SCAFFOLDING CONSTRAINTS') > 0;
