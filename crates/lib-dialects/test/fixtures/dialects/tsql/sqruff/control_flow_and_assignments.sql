IF 1 = 1
BEGIN
    PRINT 1;
END;
ELSE
BEGIN
    PRINT 2;
END;

WHILE @i < 10
BEGIN
    IF @i = 5
        BREAK;
    SET @i = @i + 1;
    CONTINUE;
END

UPDATE t
SET @old = a,
    a = @new,
    @v = b = b + 1
WHERE c = 1;

RAISERROR ('Not intended for this customer', 20, -1) WITH LOG;

SET @Errors.modify('insert <Error /> as last into (/Errors)[1]');

IF @changes.exist('Vault/*') = 1
    PRINT 'changed';

SELECT a FROM t
INTERSECT
SELECT a FROM u
OPTION (MERGE JOIN);
