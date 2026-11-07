-- MadameLabs: sobe o teto de projecao por registro de 256 KiB para 1 MiB.
-- Notas com centenas de wikilinks (como um log da base) crescem a cada alvo que passa
-- a existir e passavam de 262144 bytes. Acompanha as constantes 1_048_576 em
-- src/provider/projections/{activation,batches}.rs. Numero alto para nao colidir com
-- as migracoes do upstream (o migrador usa set_ignore_missing).
ALTER TABLE hosted_provider_record_projections
  DROP CONSTRAINT hosted_provider_record_projections_projection_bytes_check;

ALTER TABLE hosted_provider_record_projections
  ADD CONSTRAINT hosted_provider_record_projections_projection_bytes_check
  CHECK (projection_bytes >= 0 AND projection_bytes <= 1048576);
