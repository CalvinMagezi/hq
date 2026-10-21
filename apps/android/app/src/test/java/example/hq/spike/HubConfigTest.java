package example.hq.spike;

import org.junit.Test;

/** Runs the assertion-based checks in HubConfigCheck under Gradle, which turns assertions on for tests. */
public class HubConfigTest {
    @Test
    public void hubConfigChecksPass() throws Exception {
        HubConfigCheck.main(new String[0]);
    }
}
