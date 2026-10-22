EXEC Core_GetSetting 'AllowNonFuel', @SettingValue = @Allow OUTPUT

SELECT ItemNumber FROM bItem WHERE bItemTypeID = 1
UNION SELECT '{new record}' WHERE @IncludeNew = 1
ORDER BY 1;

SELECT
    'Totals:' + (
        SELECT gl.Line
        FROM grand_lines AS gl
        ORDER BY gl.SortOrder
        FOR XML PATH(''), TYPE
    ).value('.', 'varchar(max)') + '<br/>';
