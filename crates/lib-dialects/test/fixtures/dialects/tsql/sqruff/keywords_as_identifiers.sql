SELECT c.SQL, c.FilterSQL, m.definition
FROM dbo.CustomQuery AS c
INNER JOIN sys.sql_modules AS m ON m.object_id = c.ObjectID;

ALTER TABLE [dbo].[ScreenUsageDaily] SET (LOCK_ESCALATION = DISABLE);
