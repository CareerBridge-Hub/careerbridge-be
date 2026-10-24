-- Vector generation invalidates cached recommendations through the catalog trigger.
GRANT UPDATE(revision) ON catalog_revision TO careerbridge_worker;
