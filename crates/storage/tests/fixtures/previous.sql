-- Synthetic previous schema rows. Only an OS credential reference is persisted.
INSERT INTO projects VALUES (X'11111111111141118111111111111111','orders',1700000000000,1700000000000,NULL);
INSERT INTO environment_targets VALUES (X'22222222222242228222222222222222',X'11111111111141118111111111111111','dev','DEV','LOW',1,'ACTIVE');
INSERT INTO resource_bindings(id,target_id,alias,kind,host,port,tls_mode,credential_ref,policy_labels,status) VALUES (X'33333333333343338333333333333333',X'22222222222242228222222222222222','primary-db','MYSQL','fixture.invalid',3306,'VERIFY_IDENTITY','fixture-ref-1','[]','ACTIVE');
